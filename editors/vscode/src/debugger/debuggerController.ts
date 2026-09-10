import * as path from 'node:path';

import * as vscode from 'vscode';

import type { EmbeddedCompilerClient, EmbeddedCompilerFactory } from '../compilerTasks';
import { SplitScriptDebugAdapter, type DebugAdapterHost } from './debugAdapter';
import type { RuntimeLogMessage, RuntimeSnapshot } from './runtimeProtocol';
import { RuntimeViewProvider } from './runtimeView';
import { StatisticsViewProvider } from './statisticsView';
import {
    openDebugMemory,
} from './debugMemory';
import {
    SettingsMapViewProvider,
    SettingsViewProvider,
    VariablesViewProvider,
    ProcessesViewProvider,
} from './settingsViews';
import { nativePathToWasi } from './asr/wasi';

const DEBUG_TYPE = 'splitscript';
const ACTIVE_CONTEXT = 'splitscript.debug.active';

export class SplitScriptDebuggerController implements
    vscode.DebugConfigurationProvider,
    vscode.DebugAdapterDescriptorFactory,
    DebugAdapterHost,
    vscode.Disposable
{
    private readonly runtimeView = new RuntimeViewProvider();
    private readonly statisticsView = new StatisticsViewProvider();
    private readonly settingsView = new SettingsViewProvider();
    private readonly settingsMapView = new SettingsMapViewProvider();
    private readonly variablesView = new VariablesViewProvider();
    private readonly processesView = new ProcessesViewProvider();
    private readonly output = vscode.window.createOutputChannel('Auto Splitting Runtime');
    private readonly adapters = new Set<SplitScriptDebugAdapter>();
    private compilerModule: Uint8Array | undefined;
    private activeAdapter: SplitScriptDebugAdapter | undefined;
    private activeSnapshot: RuntimeSnapshot | undefined;

    public constructor(
        private readonly context: vscode.ExtensionContext,
        private readonly compilerFactory: EmbeddedCompilerFactory,
    ) {}

    public async initialize(): Promise<void> {
        this.compilerModule = await vscode.workspace.fs.readFile(vscode.Uri.joinPath(
            this.context.extensionUri,
            'dist',
            'splitscript_vscode_wasm.wasm',
        ));
        await this.setActive(false);
        this.context.subscriptions.push(
            this,
            this.runtimeView,
            this.statisticsView,
            this.settingsView,
            this.settingsMapView,
            this.variablesView,
            this.processesView,
            this.output,
            vscode.debug.registerDebugConfigurationProvider(DEBUG_TYPE, this),
            vscode.debug.registerDebugAdapterDescriptorFactory(DEBUG_TYPE, this),
            vscode.window.registerTreeDataProvider('splitscript.debugger.runtime', this.runtimeView),
            vscode.window.registerTreeDataProvider('splitscript.debugger.statistics', this.statisticsView),
            vscode.window.registerTreeDataProvider('splitscript.debugger.settings', this.settingsView),
            vscode.window.registerTreeDataProvider('splitscript.debugger.settingsMap', this.settingsMapView),
            vscode.window.registerTreeDataProvider('splitscript.debugger.variables', this.variablesView),
            vscode.window.registerTreeDataProvider('splitscript.debugger.processes', this.processesView),
            vscode.commands.registerCommand('splitscript.debug.start', async resource => {
                await this.start(resource);
            }),
            vscode.commands.registerCommand('splitscript.debug.restart', async () => {
                await this.activeAdapter?.restart();
            }),
            vscode.commands.registerCommand('splitscript.debug.stop', async () => {
                await this.activeAdapter?.stop();
            }),
            vscode.commands.registerCommand('splitscript.debug.timerStart', () => {
                this.activeAdapter?.timerCommand('start');
            }),
            vscode.commands.registerCommand('splitscript.debug.timerReset', () => {
                this.activeAdapter?.timerCommand('reset');
            }),
            vscode.commands.registerCommand('splitscript.debug.showLogs', () => this.output.show(true)),
            vscode.commands.registerCommand('splitscript.debug.editSetting', async (key: string) => {
                await this.editSetting(key);
            }),
            vscode.commands.registerCommand('splitscript.debug.clearSettings', () => {
                this.activeAdapter?.clearSettings();
            }),
            vscode.commands.registerCommand('splitscript.debug.resetStatistics', () => {
                this.activeAdapter?.resetStatistics();
            }),
            vscode.commands.registerCommand('splitscript.debug.openMemory', async () => {
                await this.openMemory();
            }),
            vscode.commands.registerCommand('splitscript.debug.openProcessMemory', async element => {
                await this.openProcessMemory(element);
            }),
        );
    }

    public resolveDebugConfiguration(
        _folder: vscode.WorkspaceFolder | undefined,
        configuration: vscode.DebugConfiguration,
    ): vscode.DebugConfiguration | undefined {
        if (!configuration.type && !configuration.request && !configuration.name) {
            const uri = activeDebugProgram();
            if (uri === undefined) {
                void vscode.window.showInformationMessage(
                    'Open a SplitScript or WebAssembly file to debug it.',
                );
                return undefined;
            }
            configuration.type = DEBUG_TYPE;
            configuration.request = 'launch';
            configuration.name = `Debug ${path.basename(uri.fsPath)}`;
            configuration.program = uri.fsPath;
            configuration.hotReload = isSplitScript(uri);
        }
        return configuration;
    }

    public resolveDebugConfigurationWithSubstitutedVariables(
        _folder: vscode.WorkspaceFolder | undefined,
        configuration: vscode.DebugConfiguration,
    ): vscode.DebugConfiguration | undefined {
        if (typeof configuration.program !== 'string' || configuration.program.length === 0) {
            void vscode.window.showErrorMessage('A .split or .wasm debug program is required.');
            return undefined;
        }
        return configuration;
    }

    public createDebugAdapterDescriptor(session: vscode.DebugSession): vscode.DebugAdapterDescriptor {
        const adapter = new SplitScriptDebugAdapter(
            this.context.asAbsolutePath('dist/runtimeWorker.js'),
            this.context.asAbsolutePath(
                `dist/native/${process.platform}-${process.arch}/splitscript_process_native.node`,
            ),
            this,
            session.id,
        );
        this.adapters.add(adapter);
        return new vscode.DebugAdapterInlineImplementation(adapter);
    }

    public createCompiler(): Promise<EmbeddedCompilerClient> {
        const module = this.compilerModule;
        if (module === undefined) {
            return Promise.reject(new Error('the SplitScript debug compiler is not initialized'));
        }
        return this.compilerFactory(module);
    }

    public snapshot(
        adapter: SplitScriptDebugAdapter,
        snapshot: RuntimeSnapshot | undefined,
    ): void {
        if (this.activeAdapter === adapter) {
            this.activeSnapshot = snapshot;
            this.runtimeView.update(snapshot);
            this.statisticsView.update(snapshot);
            this.settingsView.update(snapshot);
            this.settingsMapView.update(snapshot);
            this.variablesView.update(snapshot);
            this.processesView.update(snapshot);
        }
    }

    public log(_adapter: SplitScriptDebugAdapter, message: RuntimeLogMessage): void {
        this.output.appendLine(
            `[${message.timestamp}][${message.source}][${message.level}] ${message.message}`,
        );
    }

    public active(adapter: SplitScriptDebugAdapter): void {
        this.activeAdapter = adapter;
        this.output.clear();
        void this.setActive(true);
    }

    public stopped(adapter: SplitScriptDebugAdapter): void {
        this.adapters.delete(adapter);
        if (this.activeAdapter === adapter) {
            this.activeAdapter = undefined;
            this.activeSnapshot = undefined;
            this.runtimeView.update(undefined);
            this.statisticsView.update(undefined);
            this.settingsView.update(undefined);
            this.settingsMapView.update(undefined);
            this.variablesView.update(undefined);
            this.processesView.update(undefined);
            void this.setActive(false);
        }
    }

    public dispose(): void {
        for (const adapter of [...this.adapters]) {
            adapter.dispose();
        }
        this.adapters.clear();
        this.activeAdapter = undefined;
        this.activeSnapshot = undefined;
        this.compilerModule = undefined;
    }

    private async start(resource: unknown): Promise<void> {
        let uri = debugProgram(resource) ?? activeDebugProgram();
        if (uri === undefined) {
            const selected = await vscode.window.showOpenDialog({
                title: 'Select a SplitScript or WebAssembly Program',
                canSelectFiles: true,
                canSelectFolders: false,
                canSelectMany: false,
                filters: {
                    'Debug Programs': ['split', 'wasm'],
                    'WebAssembly Modules': ['wasm'],
                    'SplitScript Sources': ['split'],
                },
            });
            uri = selected?.[0];
        }
        if (uri === undefined) return;
        await vscode.debug.startDebugging(undefined, {
            type: DEBUG_TYPE,
            request: 'launch',
            name: `Debug ${path.basename(uri.fsPath)}`,
            program: uri.fsPath,
            hotReload: isSplitScript(uri),
        });
    }

    private async editSetting(key: string): Promise<void> {
        const adapter = this.activeAdapter;
        const widget = this.settingsView.widget(key);
        if (adapter === undefined || widget === undefined || widget.type === 'title') return;
        const current = this.settingsView.value(key);
        if (widget.type === 'bool') {
            adapter.setSetting(key, !current);
            return;
        }
        if (widget.type === 'choice') {
            const selected = await vscode.window.showQuickPick(
                widget.options.map(option => ({
                    label: option.description,
                    description: option.key,
                    key: option.key,
                    picked: option.key === current,
                })),
                { title: widget.description, placeHolder: widget.tooltip },
            );
            if (selected !== undefined) adapter.setSetting(key, selected.key);
            return;
        }
        if (widget.type === 'textInput') {
            const selected = await vscode.window.showInputBox({
                title: widget.description,
                prompt: widget.tooltip,
                value: typeof current === 'string' ? current : widget.defaultValue,
            });
            if (selected !== undefined) adapter.setSetting(key, selected);
            return;
        }
        const selected = await vscode.window.showOpenDialog({
            title: widget.description,
            canSelectFiles: true,
            canSelectFolders: false,
            canSelectMany: false,
            filters: fileFilters(widget.filters),
        });
        if (selected?.[0] !== undefined) {
            adapter.setSetting(key, nativePathToWasi(selected[0].fsPath));
        }
    }

    private async openMemory(): Promise<void> {
        const adapter = this.activeAdapter;
        if (adapter === undefined) return;
        try {
            const program = this.activeSnapshot?.program;
            const name = program === undefined
                ? 'wasm-memory'
                : `${path.basename(program, path.extname(program))}-wasm-memory`;
            await openDebugMemory(
                adapter.sessionId,
                adapter.wasmMemoryReference(),
                name,
            );
        } catch (error) {
            void vscode.window.showErrorMessage(
                `Could not open WebAssembly memory in the Hex Editor: ${asError(error).message}`,
            );
        }
    }

    private async openProcessMemory(element: unknown): Promise<void> {
        const adapter = this.activeAdapter;
        const process = this.processesView.processFor(element);
        if (adapter === undefined || process === undefined || !process.isOpen) return;
        try {
            const ranges = (await adapter.listProcessMemoryRanges(process.handle))
                .filter(range => isReadableRange(range.flags) && BigInt(range.size) > 0n);
            if (ranges.length === 0) {
                void vscode.window.showInformationMessage(
                    `PID ${process.pid} has no readable mapped memory ranges.`,
                );
                return;
            }
            const selected = await vscode.window.showQuickPick(
                ranges.map(range => {
                    const address = BigInt(range.address);
                    const size = BigInt(range.size);
                    return {
                        label: `${hex(address)} – ${hex(address + size)}`,
                        description: memoryPermissions(range.flags),
                        detail: formatBytes(size),
                        range,
                    };
                }),
                {
                    title: `Open Memory for PID ${process.pid}`,
                    placeHolder: 'Select a readable mapped range',
                    matchOnDescription: true,
                    matchOnDetail: true,
                },
            );
            if (selected === undefined) return;
            const address = BigInt(selected.range.address);
            const executable = process.path?.split(/[\\/]/).at(-1) ?? `pid-${process.pid}`;
            await openDebugMemory(
                adapter.sessionId,
                adapter.processMemoryReference(process.handle),
                `${executable}-${address.toString(16)}`,
                selected.range.address,
            );
        } catch (error) {
            void vscode.window.showErrorMessage(
                `Could not open process memory in the Hex Editor: ${asError(error).message}`,
            );
        }
    }

    private setActive(active: boolean): Thenable<unknown> {
        return vscode.commands.executeCommand('setContext', ACTIVE_CONTEXT, active);
    }
}

function activeDebugProgram(): vscode.Uri | undefined {
    const document = vscode.window.activeTextEditor?.document;
    const textUri = document?.uri;
    if (textUri !== undefined && isDebugProgram(textUri)) return textUri;
    return debugProgram(vscode.window.tabGroups.activeTabGroup.activeTab?.input);
}

function debugProgram(value: unknown): vscode.Uri | undefined {
    if (value instanceof vscode.Uri) return isDebugProgram(value) ? value : undefined;
    if (typeof value !== 'object' || value === null || !('uri' in value)) return undefined;
    const uri = value.uri;
    return uri instanceof vscode.Uri && isDebugProgram(uri) ? uri : undefined;
}

function isDebugProgram(uri: vscode.Uri): boolean {
    if (uri.scheme !== 'file') return false;
    const extension = path.extname(uri.fsPath).toLowerCase();
    return extension === '.split' || extension === '.wasm';
}

function isSplitScript(uri: vscode.Uri): boolean {
    return path.extname(uri.fsPath).toLowerCase() === '.split';
}

function asError(error: unknown): Error {
    return error instanceof Error ? error : new Error(String(error));
}

function isReadableRange(flags: string): boolean {
    return (BigInt(flags) & 2n) !== 0n;
}

function memoryPermissions(flags: string): string {
    const value = BigInt(flags);
    return [
        (value & 2n) !== 0n ? 'r' : '-',
        (value & 4n) !== 0n ? 'w' : '-',
        (value & 8n) !== 0n ? 'x' : '-',
    ].join('');
}

function hex(value: bigint): string {
    return `0x${value.toString(16).padStart(8, '0')}`;
}

function formatBytes(bytes: bigint): string {
    const kibibyte = 1_024n;
    const mebibyte = kibibyte * kibibyte;
    const gibibyte = mebibyte * kibibyte;
    if (bytes >= gibibyte) return `${formatRatio(bytes, gibibyte)} GiB`;
    if (bytes >= mebibyte) return `${formatRatio(bytes, mebibyte)} MiB`;
    if (bytes >= kibibyte) return `${formatRatio(bytes, kibibyte)} KiB`;
    return `${bytes} B`;
}

function formatRatio(value: bigint, unit: bigint): string {
    const tenths = value * 10n / unit;
    return `${tenths / 10n}.${tenths % 10n}`;
}

function fileFilters(
    filters: Extract<import('./runtimeProtocol').SettingWidgetSnapshot, { type: 'fileSelect' }>['filters'],
): Record<string, string[]> | undefined {
    const result: Record<string, string[]> = {};
    for (const filter of filters) {
        if (filter.type !== 'name') continue;
        const extensions = filter.pattern.split(/\s+/)
            .map(pattern => /^\*\.([^*?\[\]{}]+)$/.exec(pattern)?.[1])
            .filter((extension): extension is string => extension !== undefined);
        if (extensions.length > 0) result[filter.description ?? 'Files'] = extensions;
    }
    return Object.keys(result).length === 0 ? undefined : result;
}
