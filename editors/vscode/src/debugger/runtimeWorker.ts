import { parentPort } from 'node:worker_threads';

import { GuestMemory } from './asr/memory';
import { neutralImport } from './asr/neutralImports';
import { DebuggerTimer } from './asr/timer';
import type {
    RuntimeLogMessage,
    RuntimeRequest,
    RuntimeResponse,
    RuntimeSnapshot,
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
        host = new RuntimeHost(message.program);
        void host.launch(message.wasm).catch(fail);
    } else if (message.type === 'timerCommand') {
        host?.timerCommand(message.command);
    }
});

class RuntimeHost {
    private readonly memory = new GuestMemory();
    private readonly timer: DebuggerTimer;
    private status: RuntimeSnapshot['status'] = 'starting';
    private tickRateHz = 120;
    private tickCount = 0;
    private averageTickMilliseconds = 0;
    private slowestTickMilliseconds = 0;
    private lastSnapshotTime = 0;
    private initialize: (() => void) | undefined;
    private update: (() => void) | undefined;
    private tickTimer: NodeJS.Timeout | undefined;

    public constructor(private readonly program: string) {
        this.timer = new DebuggerTimer(
            () => this.emitSnapshot(true),
            message => this.emitLog(message),
        );
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
                message: `Using neutral Milestone 1 stubs for: ${unsupportedImports.join(', ')}`,
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
            this.status = 'trapped';
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
        this.emitSnapshot(false);
        this.scheduleTick(1_000 / this.tickRateHz);
    }

    private createImports(
        entries: WebAssembly.ModuleImportDescriptor[],
        unsupported: string[],
    ): WebAssembly.Imports {
        const available = this.availableEnvImports();
        const imports: Record<string, Record<string, WebAssembly.ImportValue>> = {};
        for (const entry of entries) {
            if (entry.kind !== 'function') {
                throw new Error(`unsupported import ${entry.module}.${entry.name} (${entry.kind})`);
            }
            const namespace = imports[entry.module] ??= {};
            const exact = entry.module === 'env' ? available[entry.name] : undefined;
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
