import { parentPort } from 'node:worker_threads';

import { GuestMemory } from './asr/memory';
import { neutralImport } from './asr/neutralImports';
import { loadNativeProcessBridge, ProcessHost } from './asr/process';
import { SettingsHost } from './asr/settings';
import { DebuggerTimer } from './asr/timer';
import { WasiHost } from './asr/wasi';
import type {
    RuntimeLogMessage,
    RuntimeRequest,
    RuntimeResponse,
    RuntimeSnapshot,
    SettingMapSnapshot,
} from './runtimeProtocol';

const port = parentPort;
if (port === null) {
    throw new Error('the ASR runtime must run in a Node worker');
}
const workerPort = port;
let host: RuntimeHost | undefined;

workerPort.on('message', (message: RuntimeRequest) => {
    if (message.type === 'launch') {
        if (host !== undefined) {
            fail(new Error('this runtime worker has already launched a module'));
            return;
        }
        host = new RuntimeHost(
            message.program,
            message.scriptPath,
            message.settings,
            message.nativeModulePath,
        );
        void host.launch(message.wasm).catch(fail);
    } else if (message.type === 'timerCommand') {
        host?.timerCommand(message.command);
    } else if (message.type === 'setSetting') {
        host?.setSetting(message.key, message.value);
    } else if (message.type === 'clearSettings') {
        host?.clearSettings();
    } else if (message.type === 'shutdown') {
        host?.dispose();
        host = undefined;
        workerPort.postMessage({ type: 'stopped' } satisfies RuntimeResponse);
    }
});

class RuntimeHost {
    private readonly memory = new GuestMemory();
    private readonly timer: DebuggerTimer;
    private readonly settings: SettingsHost;
    private readonly wasi: WasiHost;
    private readonly processes: ProcessHost | undefined;
    private readonly nativeBridgeError: string | undefined;
    private status: RuntimeSnapshot['status'] = 'starting';
    private tickRateHz = 120;
    private tickCount = 0;
    private averageTickMilliseconds = 0;
    private slowestTickMilliseconds = 0;
    private lastSnapshotTime = 0;
    private initialize: (() => void) | undefined;
    private update: (() => void) | undefined;
    private tickTimer: NodeJS.Timeout | undefined;

    public constructor(
        private readonly program: string,
        scriptPath: string | undefined,
        initialSettings: SettingMapSnapshot | undefined,
        nativeModulePath: string | undefined,
    ) {
        this.timer = new DebuggerTimer(
            () => this.emitSnapshot(true),
            message => this.emitLog(message),
        );
        this.settings = new SettingsHost(this.memory, initialSettings);
        this.wasi = new WasiHost(this.memory, scriptPath, message => this.emitLog(message));
        let processes: ProcessHost | undefined;
        let nativeBridgeError: string | undefined;
        if (nativeModulePath !== undefined) {
            try {
                processes = new ProcessHost(
                    this.memory,
                    loadNativeProcessBridge(nativeModulePath),
                    message => this.emitLog(message),
                );
            } catch (error) {
                nativeBridgeError = error instanceof Error ? error.message : String(error);
            }
        }
        this.processes = processes;
        this.nativeBridgeError = nativeBridgeError;
    }

    public async launch(bytes: ArrayBuffer): Promise<void> {
        const module = await WebAssembly.compile(bytes);
        const moduleImports = WebAssembly.Module.imports(module);
        const unsupportedImports: string[] = [];
        const imports = this.createImports(moduleImports, unsupportedImports);
        const instance = await WebAssembly.instantiate(module, imports);
        const memory = instance.exports.memory;
        if (!(memory instanceof WebAssembly.Memory)) {
            throw new Error('the ASR module does not export its memory');
        }
        const update = instance.exports.update;
        if (typeof update !== 'function') {
            throw new Error('the ASR module does not export update');
        }
        this.memory.bind(memory);
        const initialize = instance.exports._initialize ?? instance.exports._start;
        this.initialize = typeof initialize === 'function'
            ? initialize as () => void
            : undefined;
        this.update = update as () => void;
        this.status = 'running';
        workerPort.postMessage({ type: 'ready', unsupportedImports } satisfies RuntimeResponse);
        if (unsupportedImports.length > 0) {
            this.emitLog({
                source: 'runtime',
                level: 'warning',
                message: `Using unavailable-host stubs for: ${unsupportedImports.join(', ')}`,
            });
        }
        if (unsupportedImports.some(name => name.startsWith('env.process_'))
            && this.nativeBridgeError !== undefined) {
            this.emitLog({
                source: 'runtime',
                level: 'error',
                message: `Could not load the native process bridge: ${this.nativeBridgeError}`,
            });
        }
        this.emitLog({
            source: 'runtime',
            level: 'info',
            message: `Loaded ${this.program}`,
        });
        this.emitSnapshot(true);
        this.scheduleTick(0);
    }

    public timerCommand(command: 'start' | 'reset'): void {
        if (command === 'start') {
            this.timer.start();
        } else {
            this.timer.reset();
        }
    }

    public setSetting(key: string, value: boolean | string): void {
        this.settings.set(key, value);
        this.emitSnapshot(true);
    }

    public clearSettings(): void {
        this.settings.clear();
        this.emitSnapshot(true);
    }

    public dispose(): void {
        this.status = 'trapped';
        if (this.tickTimer !== undefined) {
            clearTimeout(this.tickTimer);
            this.tickTimer = undefined;
        }
        this.processes?.dispose();
        this.wasi.dispose();
    }

    private scheduleTick(delay: number): void {
        this.tickTimer = setTimeout(() => this.tick(), delay);
    }

    private tick(): void {
        const update = this.update;
        if (this.status !== 'running' || update === undefined) {
            return;
        }
        const started = performance.now();
        try {
            const initialize = this.initialize;
            this.initialize = undefined;
            initialize?.();
            update();
        } catch (error) {
            this.dispose();
            this.emitSnapshot(true);
            fail(error);
            return;
        }
        const duration = performance.now() - started;
        this.tickCount += 1;
        this.averageTickMilliseconds = this.tickCount === 1
            ? duration
            : this.averageTickMilliseconds * 0.999 + duration * 0.001;
        this.slowestTickMilliseconds = Math.max(this.slowestTickMilliseconds, duration);
        this.emitSnapshot(
            this.settings.consumeChanged() || (this.processes?.consumeChanged() ?? false),
        );
        this.scheduleTick(1_000 / this.tickRateHz);
    }

    private createImports(
        entries: WebAssembly.ModuleImportDescriptor[],
        unsupported: string[],
    ): WebAssembly.Imports {
        const available = this.availableEnvImports();
        const wasi = this.wasi.imports();
        const imports: Record<string, Record<string, WebAssembly.ImportValue>> = {};
        for (const entry of entries) {
            if (entry.kind !== 'function') {
                throw new Error(`unsupported import ${entry.module}.${entry.name} (${entry.kind})`);
            }
            const namespace = imports[entry.module] ??= {};
            const exact = entry.module === 'env'
                ? available[entry.name]
                : entry.module === 'wasi_snapshot_preview1'
                    ? wasi[entry.name]
                    : undefined;
            if (exact !== undefined) {
                namespace[entry.name] = exact;
            } else {
                const qualified = `${entry.module}.${entry.name}`;
                unsupported.push(qualified);
                namespace[entry.name] = neutralImport(qualified);
            }
        }
        return imports;
    }

    private availableEnvImports(): Record<string, WebAssembly.ImportValue> {
        return {
            ...this.settings.imports(),
            ...this.processes?.imports(),
            timer_get_state: () => this.timer.stateNumber(),
            timer_current_split_index: () => this.timer.currentSplitIndex(),
            timer_segment_splitted: (index: bigint) => this.timer.segmentSplitted(index),
            timer_start: () => this.timer.start(),
            timer_split: () => this.timer.split(),
            timer_skip_split: () => this.timer.skipSplit(),
            timer_undo_split: () => this.timer.undoSplit(),
            timer_reset: () => this.timer.reset(),
            timer_set_variable: (
                namePointer: number,
                nameLength: number,
                valuePointer: number,
                valueLength: number,
            ) => this.timer.setVariable(
                this.memory.readString(namePointer, nameLength),
                this.memory.readString(valuePointer, valueLength),
            ),
            timer_set_game_time: (seconds: bigint, nanoseconds: number) => {
                this.timer.setGameTime(seconds, nanoseconds);
            },
            timer_pause_game_time: () => this.timer.pauseGameTime(),
            timer_resume_game_time: () => this.timer.resumeGameTime(),
            runtime_set_tick_rate: (ticksPerSecond: number) => {
                if (!Number.isFinite(ticksPerSecond) || ticksPerSecond <= 0) {
                    throw new WebAssembly.RuntimeError('the tick rate needs to be finite and larger than 0');
                }
                this.tickRateHz = ticksPerSecond;
                this.emitLog({
                    source: 'runtime',
                    level: 'debug',
                    message: `New Tick Rate: ${ticksPerSecond}`,
                });
                this.emitSnapshot(true);
            },
            runtime_print_message: (pointer: number, length: number) => {
                this.emitLog({
                    source: 'autoSplitter',
                    level: 'info',
                    message: this.memory.readString(pointer, length),
                });
            },
            runtime_get_os: (pointer: number, lengthPointer: number) => {
                return this.memory.writeHostString(pointer, lengthPointer, asrOperatingSystem());
            },
            runtime_get_arch: (pointer: number, lengthPointer: number) => {
                return this.memory.writeHostString(pointer, lengthPointer, asrArchitecture());
            },
        };
    }

    private emitSnapshot(force: boolean): void {
        const now = performance.now();
        if (!force && now - this.lastSnapshotTime < 100) {
            return;
        }
        this.lastSnapshotTime = now;
        workerPort.postMessage({
            type: 'snapshot',
            snapshot: {
                status: this.status,
                program: this.program,
                tickRateHz: this.tickRateHz,
                tickCount: this.tickCount,
                averageTickMilliseconds: this.averageTickMilliseconds,
                slowestTickMilliseconds: this.slowestTickMilliseconds,
                memoryBytes: this.memory.byteLength(),
                timer: this.timer.snapshot(),
                settings: this.settings.snapshot(),
                processes: this.processes?.snapshot() ?? [],
            },
        } satisfies RuntimeResponse);
    }

    private emitLog(message: Omit<RuntimeLogMessage, 'type' | 'timestamp'>): void {
        workerPort.postMessage({
            type: 'log',
            timestamp: new Date().toISOString(),
            ...message,
        } satisfies RuntimeResponse);
    }
}

function fail(error: unknown): void {
    workerPort.postMessage({
        type: 'failure',
        message: error instanceof Error ? error.message : String(error),
        stack: error instanceof Error ? error.stack : undefined,
    } satisfies RuntimeResponse);
}

function asrOperatingSystem(): string {
    switch (process.platform) {
        case 'win32': return 'windows';
        case 'darwin': return 'macos';
        default: return process.platform;
    }
}

function asrArchitecture(): string {
    switch (process.arch) {
        case 'x64': return 'x86_64';
        case 'ia32': return 'x86';
        case 'arm64': return 'aarch64';
        default: return process.arch;
    }
}
