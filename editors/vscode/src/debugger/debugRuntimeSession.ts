import * as vscode from 'vscode';

import {
    compilerServiceProtocolVersion,
    type EmbeddedCompileResponse,
} from '../embeddedCompiler';
import type { EmbeddedCompilerClient } from '../compilerTasks';
import { RuntimeClient } from './runtimeClient';
import type { RuntimeLogMessage, RuntimeSnapshot } from './runtimeProtocol';

export interface SplitScriptLaunchConfiguration extends vscode.DebugConfiguration {
    type: 'splitscript';
    request: 'launch';
    program: string;
    hotReload?: boolean;
    scriptPath?: string;
}

export interface DebugRuntimeSessionCallbacks {
    snapshot(snapshot: RuntimeSnapshot | undefined): void;
    log(message: RuntimeLogMessage): void;
    failure(error: Error): void;
}

export class DebugRuntimeSession implements vscode.Disposable {
    private readonly runtime: RuntimeClient;
    private compiler: EmbeddedCompilerClient | undefined;
    private programUri: vscode.Uri | undefined;
    private hotReload = true;
    private scriptPath: string | undefined;
    private reloadChain = Promise.resolve();
    private readonly saveSubscription: vscode.Disposable;
    private stopped = false;
    private suppressSaveReload = false;

    public constructor(
        workerPath: string,
        nativeModulePath: string,
        private readonly createCompiler: () => Promise<EmbeddedCompilerClient>,
        private readonly callbacks: DebugRuntimeSessionCallbacks,
    ) {
        this.runtime = new RuntimeClient(workerPath, nativeModulePath, callbacks);
        this.saveSubscription = vscode.workspace.onDidSaveTextDocument(document => {
            if (
                this.hotReload
                && !this.suppressSaveReload
                && this.programUri !== undefined
                && document.uri.toString() === this.programUri.toString()
            ) {
                void this.reload().catch(error => {
                    this.callbacks.log(runtimeLog(
                        'error',
                        `Hot reload failed: ${asError(error).message}`,
                    ));
                });
            }
        });
    }

    public async launch(configuration: SplitScriptLaunchConfiguration): Promise<void> {
        if (!vscode.workspace.isTrusted) {
            throw new Error('SplitScript debugging requires a trusted workspace.');
        }
        const uri = programUri(configuration.program);
        if (uri.scheme !== 'file') {
            throw new Error('SplitScript debugging currently requires a local file.');
        }
        this.hotReload = configuration.hotReload !== false;
        this.scriptPath = configuration.scriptPath;
        this.compiler = await this.createCompiler();
        const artifact = await this.buildArtifact(uri);
        this.programUri = uri;
        await this.runtime.launch(artifact, uri.fsPath, this.scriptPath);
    }

    public reload(): Promise<void> {
        const uri = this.programUri;
        if (uri === undefined || this.stopped) {
            return Promise.resolve();
        }
        const operation = this.reloadChain.then(async () => {
            this.callbacks.log(runtimeLog('debug', `Rebuilding ${uri.fsPath}`));
            const artifact = await this.buildArtifact(uri);
            this.callbacks.snapshot(undefined);
            await this.runtime.launch(artifact, uri.fsPath, this.scriptPath);
            this.callbacks.log(runtimeLog('info', `Reloaded ${uri.fsPath}`));
        });
        this.reloadChain = operation.catch(() => {});
        return operation;
    }

    public timerCommand(command: 'start' | 'reset'): void {
        this.runtime.timerCommand(command);
    }

    public setSetting(key: string, value: boolean | string): void {
        this.runtime.setSetting(key, value);
    }

    public clearSettings(): void {
        this.runtime.clearSettings();
    }

    public resetStatistics(): void {
        this.runtime.resetStatistics();
    }

    public dumpMemory(): Promise<Uint8Array> {
        return this.runtime.dumpMemory();
    }

    public async stop(): Promise<void> {
        if (this.stopped) {
            return;
        }
        this.stopped = true;
        this.saveSubscription.dispose();
        this.compiler?.dispose();
        this.compiler = undefined;
        await this.runtime.terminate();
        this.callbacks.snapshot(undefined);
    }

    public dispose(): void {
        void this.stop();
    }

    private async buildArtifact(uri: vscode.Uri): Promise<Uint8Array> {
        if (uri.path.toLowerCase().endsWith('.wasm')) {
            return vscode.workspace.fs.readFile(uri);
        }
        if (!uri.path.toLowerCase().endsWith('.split')) {
            throw new Error('The debug program must be a .split or .wasm file.');
        }
        const compiler = this.compiler;
        if (compiler === undefined) {
            throw new Error('the embedded debug compiler is not initialized');
        }
        const document = await vscode.workspace.openTextDocument(uri);
        if (document.isDirty) {
            this.suppressSaveReload = true;
            try {
                if (!await document.save()) {
                    throw new Error(`Could not save ${uri.fsPath} before compiling it.`);
                }
            } finally {
                this.suppressSaveReload = false;
            }
        }
        const response = await compiler.compile({
            protocolVersion: compilerServiceProtocolVersion,
            uri: uri.toString(),
            sourcePath: uri.fsPath,
            revision: document.version,
            source: document.getText(),
            profile: 'debug',
        });
        this.reportDiagnostics(response);
        if (response.artifact === undefined) {
            throw new Error(`SplitScript debug compilation failed for ${uri.fsPath}.`);
        }
        return response.artifact;
    }

    private reportDiagnostics(response: EmbeddedCompileResponse): void {
        for (const diagnostic of response.diagnostics) {
            const level = diagnostic.severity === 'error'
                ? 'error'
                : diagnostic.severity === 'warning'
                    ? 'warning'
                    : 'info';
            this.callbacks.log(runtimeLog(
                level,
                `${diagnostic.severity}[${diagnostic.code}]: ${diagnostic.message}`,
            ));
        }
    }
}

function programUri(program: string): vscode.Uri {
    return /^[a-z][a-z0-9+.-]*:\/\//i.test(program)
        ? vscode.Uri.parse(program)
        : vscode.Uri.file(program);
}

function runtimeLog(
    level: RuntimeLogMessage['level'],
    message: string,
): RuntimeLogMessage {
    return {
        type: 'log',
        timestamp: new Date().toISOString(),
        source: 'runtime',
        level,
        message,
    };
}

function asError(error: unknown): Error {
    return error instanceof Error ? error : new Error(String(error));
}
