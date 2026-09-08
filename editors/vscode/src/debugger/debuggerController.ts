import * as vscode from 'vscode';

import type { EmbeddedCompilerClient, EmbeddedCompilerFactory } from '../compilerTasks';
import { SplitScriptDebugAdapter, type DebugAdapterHost } from './debugAdapter';
import type { RuntimeLogMessage, RuntimeSnapshot } from './runtimeProtocol';
import { RuntimeViewProvider } from './runtimeView';

const DEBUG_TYPE = 'splitscript';
const ACTIVE_CONTEXT = 'splitscript.debug.active';

export class SplitScriptDebuggerController implements
    vscode.DebugConfigurationProvider,
    vscode.DebugAdapterDescriptorFactory,
    DebugAdapterHost,
    vscode.Disposable
{
    private readonly runtimeView = new RuntimeViewProvider();
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
            this.output,
            vscode.debug.registerDebugConfigurationProvider(DEBUG_TYPE, this),
            vscode.debug.registerDebugAdapterDescriptorFactory(DEBUG_TYPE, this),
            vscode.window.registerTreeDataProvider('splitscript.debug.runtime', this.runtimeView),
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

    private setActive(active: boolean): Thenable<unknown> {
        return vscode.commands.executeCommand('setContext', ACTIVE_CONTEXT, active);
    }
}
