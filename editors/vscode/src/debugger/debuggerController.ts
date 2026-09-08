import * as vscode from 'vscode';

import type { EmbeddedCompilerClient, EmbeddedCompilerFactory } from '../compilerTasks';
import { SplitScriptDebugAdapter, type DebugAdapterHost } from './debugAdapter';
import type { RuntimeLogMessage, RuntimeSnapshot } from './runtimeProtocol';
import { RuntimeViewProvider } from './runtimeView';
import {
    SettingsMapViewProvider,
    SettingsViewProvider,
    VariablesViewProvider,
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
    private readonly settingsView = new SettingsViewProvider();
    private readonly settingsMapView = new SettingsMapViewProvider();
    private readonly variablesView = new VariablesViewProvider();
    private readonly output = vscode.window.createOutputChannel('SplitScript Runtime');
    private readonly adapters = new Set<SplitScriptDebugAdapter>();
    private compilerModule: Uint8Array | undefined;
    private activeAdapter: SplitScriptDebugAdapter | undefined;

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
            this.settingsView,
            this.settingsMapView,
            this.variablesView,
            this.output,
            vscode.debug.registerDebugConfigurationProvider(DEBUG_TYPE, this),
            vscode.debug.registerDebugAdapterDescriptorFactory(DEBUG_TYPE, this),
            vscode.window.registerTreeDataProvider('splitscript.debug.runtime', this.runtimeView),
            vscode.window.registerTreeDataProvider('splitscript.debug.settings', this.settingsView),
            vscode.window.registerTreeDataProvider('splitscript.debug.settingsMap', this.settingsMapView),
            vscode.window.registerTreeDataProvider('splitscript.debug.variables', this.variablesView),
            vscode.commands.registerCommand('splitscript.debug.start', async () => this.start()),
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
        );
    }

    public resolveDebugConfiguration(
        _folder: vscode.WorkspaceFolder | undefined,
        configuration: vscode.DebugConfiguration,
    ): vscode.DebugConfiguration | undefined {
        if (!configuration.type && !configuration.request && !configuration.name) {
            const editor = vscode.window.activeTextEditor;
            if (editor?.document.languageId !== 'splitscript') {
                void vscode.window.showInformationMessage('Open a SplitScript file to debug it.');
                return undefined;
            }
            configuration.type = DEBUG_TYPE;
            configuration.request = 'launch';
            configuration.name = 'Debug SplitScript';
            configuration.program = editor.document.uri.fsPath;
            configuration.hotReload = true;
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

    public createDebugAdapterDescriptor(): vscode.DebugAdapterDescriptor {
        const adapter = new SplitScriptDebugAdapter(
            this.context.asAbsolutePath('dist/runtimeWorker.js'),
            this,
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
            this.runtimeView.update(snapshot);
            this.settingsView.update(snapshot);
            this.settingsMapView.update(snapshot);
            this.variablesView.update(snapshot);
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
            this.runtimeView.update(undefined);
            this.settingsView.update(undefined);
            this.settingsMapView.update(undefined);
            this.variablesView.update(undefined);
            void this.setActive(false);
        }
    }

    public dispose(): void {
        for (const adapter of [...this.adapters]) {
            adapter.dispose();
        }
        this.adapters.clear();
        this.activeAdapter = undefined;
        this.compilerModule = undefined;
    }

    private async start(): Promise<void> {
        const editor = vscode.window.activeTextEditor;
        if (editor?.document.languageId !== 'splitscript') {
            void vscode.window.showInformationMessage('Open a SplitScript file to debug it.');
            return;
        }
        await vscode.debug.startDebugging(undefined, {
            type: DEBUG_TYPE,
            request: 'launch',
            name: `Debug ${editor.document.fileName}`,
            program: editor.document.uri.fsPath,
            hotReload: true,
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

    private setActive(active: boolean): Thenable<unknown> {
        return vscode.commands.executeCommand('setContext', ACTIVE_CONTEXT, active);
    }
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
