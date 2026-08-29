import * as vscode from 'vscode';
import {
    compilerServiceProtocolVersion,
    type EmbeddedBuildProfile,
    type EmbeddedCompileResponse,
    type EmbeddedServiceDiagnostic,
} from './embeddedCompiler';
import { errorMessage } from './paths';
import { resolveSavedScript, type SavedScriptFailure } from './savedScript';
import { ExclusiveTaskState } from './taskState';

export interface EmbeddedCompilerClient {
    compile(request: import('./embeddedCompiler').EmbeddedCompileRequest): Promise<EmbeddedCompileResponse>;
    cancelCurrentCompilation(): void;
    dispose(): void;
}

export type EmbeddedCompilerFactory = (
    moduleBytes: Uint8Array,
) => Promise<EmbeddedCompilerClient>;

interface CompileSnapshot {
    uri: vscode.Uri;
    key: string;
    name: string;
    revision: number;
    source: string;
}

interface ReleaseTask {
    kind: 'release';
    completion: Promise<EmbeddedCompileResponse>;
}

interface WatchTask {
    kind: 'watch';
    inputKey: string;
    inputName: string;
    output: vscode.Uri;
    pending: CompileSnapshot | undefined;
    compiling: boolean;
}

type CompilerTask = ReleaseTask | WatchTask;

export class CompilerTaskController implements vscode.Disposable {
    private readonly tasks = new ExclusiveTaskState<CompilerTask>();
    private readonly outputChannel = vscode.window.createOutputChannel('SplitScript Compiler');
    private readonly watchStatus = vscode.window.createStatusBarItem(
        vscode.StatusBarAlignment.Left,
        100,
    );
    private readonly saveSubscription: vscode.Disposable;
    private compiler: EmbeddedCompilerClient | undefined;
    private temporaryCounter = 0;

    public constructor(
        private readonly context: vscode.ExtensionContext,
        private readonly createCompiler: EmbeddedCompilerFactory,
    ) {
        this.watchStatus.command = 'splitscript.stopDebugWatch';
        this.saveSubscription = vscode.workspace.onDidSaveTextDocument(document => {
            const active = this.tasks.current;
            if (
                active?.kind === 'watch'
                && document.languageId === 'splitscript'
                && document.uri.toString() === active.inputKey
            ) {
                this.queueWatchBuild(active, this.snapshot(document));
            }
        });
    }

    public async initialize(): Promise<void> {
        await this.setWatchContext(false);
        const moduleUri = vscode.Uri.joinPath(
            this.context.extensionUri,
            'dist',
            'splitscript_vscode_wasm.wasm',
        );
        const moduleBytes = await vscode.workspace.fs.readFile(moduleUri);
        this.compiler = await this.createCompiler(moduleBytes);
    }

    public async buildRelease(): Promise<void> {
        if (this.tasks.current?.kind === 'release') {
            void vscode.window.showInformationMessage(
                'A SplitScript release build is already running.',
            );
            return;
        }
        if (this.tasks.current?.kind === 'watch') {
            const action = await vscode.window.showWarningMessage(
                'Stop the debug watcher before building a release module.',
                'Stop Watch and Build Release',
            );
            if (action !== 'Stop Watch and Build Release') {
                return;
            }
            await this.stopDebugWatch(false);
        }
        const document = await this.savedActiveScript();
        if (document === undefined) {
            return;
        }

        const snapshot = this.snapshot(document);
        const output = outputUri(snapshot.uri);
        this.beginOutput('release', snapshot, output);
        const completion = this.compile(snapshot, 'release');
        const task: ReleaseTask = { kind: 'release', completion };
        if (!this.tasks.begin(task)) {
            throw new Error('compiler task ownership changed before the release build started');
        }

        let response: EmbeddedCompileResponse;
        try {
            response = await vscode.window.withProgress(
                {
                    location: vscode.ProgressLocation.Notification,
                    title: `Building ${snapshot.name} for release`,
                },
                () => completion,
            );
        } catch (error) {
            this.tasks.finish(task);
            this.reportWorkerError('Release build failed', error);
            return;
        }

        if (document.version !== response.revision) {
            this.tasks.finish(task);
            this.outputChannel.appendLine('Discarded the build because the document changed.');
            void vscode.window.showWarningMessage(
                `${snapshot.name} changed during the release build. Build it again to publish the latest revision.`,
            );
            return;
        }
        this.appendDiagnostics(snapshot, response.diagnostics);
        if (response.artifact === undefined) {
            this.tasks.finish(task);
            this.outputChannel.show(true);
            void vscode.window.showErrorMessage(
                `SplitScript release build failed. See the compiler output for details.`,
            );
            return;
        }

        try {
            await this.replaceOutput(output, response.artifact);
        } catch (error) {
            this.tasks.finish(task);
            this.reportWorkerError('Could not write the release module', error);
            return;
        }
        this.tasks.finish(task);
        this.outputChannel.appendLine(`Created ${output.toString(true)}`);
        void vscode.window.showInformationMessage(
            `Built release module ${resourceName(output)}.`,
            'Reveal Output',
        ).then(async action => {
            if (action !== 'Reveal Output') {
                return;
            }
            if (output.scheme === 'file') {
                await vscode.commands.executeCommand('revealFileInOS', output);
            } else {
                await vscode.commands.executeCommand('vscode.open', output);
            }
        });
    }

    public async startDebugWatch(): Promise<void> {
        if (this.tasks.current?.kind === 'release') {
            void vscode.window.showInformationMessage(
                'Wait for the SplitScript release build to finish before starting debug watch.',
            );
            return;
        }
        const document = await this.savedActiveScript();
        if (document === undefined) {
            return;
        }
        const snapshot = this.snapshot(document);
        const active = this.tasks.current;
        if (active?.kind === 'watch') {
            void vscode.window.showInformationMessage(
                active.inputKey === snapshot.key
                    ? `${active.inputName} is already being watched.`
                    : `Debug watch is already active for ${active.inputName}. Stop it before watching another script.`,
            );
            return;
        }

        const output = outputUri(snapshot.uri);
        const task: WatchTask = {
            kind: 'watch',
            inputKey: snapshot.key,
            inputName: snapshot.name,
            output,
            pending: undefined,
            compiling: false,
        };
        if (!this.tasks.begin(task)) {
            throw new Error('compiler task ownership changed before debug watch started');
        }
        this.outputChannel.clear();
        this.outputChannel.appendLine(
            `Watching ${snapshot.uri.toString(true)} with the embedded debug compiler`,
        );
        this.outputChannel.appendLine('');
        this.watchStatus.text = `$(sync~spin) Debug watch: ${snapshot.name}`;
        this.watchStatus.tooltip = 'SplitScript debug watch is active. Click to stop.';
        this.watchStatus.show();
        await this.setWatchContext(true);
        this.queueWatchBuild(task, snapshot);

        void vscode.window.showInformationMessage(
            `Debug watch started for ${snapshot.name}. Saving the file recompiles ${resourceName(output)}.`,
        );
    }

    public async stopDebugWatch(showNotification: boolean): Promise<void> {
        const active = this.tasks.current;
        if (active?.kind !== 'watch') {
            if (showNotification) {
                void vscode.window.showInformationMessage('No SplitScript debug watch is active.');
            }
            return;
        }
        active.pending = undefined;
        if (active.compiling) {
            this.compiler?.cancelCurrentCompilation();
        }
        await this.clearWatch(active);
        this.outputChannel.appendLine('Debug watch stopped.');
        if (showNotification) {
            void vscode.window.showInformationMessage(
                `Stopped debug watch for ${active.inputName}.`,
            );
        }
    }

    public dispose(): void {
        void this.stopDebugWatch(false);
        this.compiler?.dispose();
        this.compiler = undefined;
        this.saveSubscription.dispose();
        this.watchStatus.dispose();
        this.outputChannel.dispose();
    }

    private queueWatchBuild(task: WatchTask, snapshot: CompileSnapshot): void {
        if (this.tasks.current !== task) {
            return;
        }
        task.pending = snapshot;
        if (task.compiling) {
            this.compiler?.cancelCurrentCompilation();
        } else {
            void this.drainWatchBuilds(task);
        }
    }

    private async drainWatchBuilds(task: WatchTask): Promise<void> {
        task.compiling = true;
        try {
            while (this.tasks.current === task && task.pending !== undefined) {
                const snapshot = task.pending;
                task.pending = undefined;
                this.watchStatus.text = `$(sync~spin) Debug watch: ${task.inputName}`;
                let response: EmbeddedCompileResponse;
                try {
                    response = await this.compile(snapshot, 'debug');
                } catch (error) {
                    if (
                        compilerErrorCode(error) === 'cancelled'
                        && this.tasks.current === task
                        && task.pending !== undefined
                    ) {
                        this.outputChannel.appendLine(
                            `Cancelled revision ${snapshot.revision}; a newer save is queued.`,
                        );
                        continue;
                    }
                    throw error;
                }
                if (this.tasks.current !== task) {
                    return;
                }
                if (task.pending !== undefined) {
                    this.outputChannel.appendLine(
                        `Discarded revision ${response.revision}; a newer save is queued.`,
                    );
                    continue;
                }
                this.appendDiagnostics(snapshot, response.diagnostics);
                if (response.artifact === undefined) {
                    this.outputChannel.show(true);
                    this.watchStatus.text = `$(error) Debug watch: ${task.inputName}`;
                    continue;
                }
                await this.replaceOutput(task.output, response.artifact);
                this.outputChannel.appendLine(
                    `Built revision ${response.revision} -> ${task.output.toString(true)}`,
                );
                this.watchStatus.text = `$(check) Debug watch: ${task.inputName}`;
            }
        } catch (error) {
            if (this.tasks.current === task) {
                this.reportWorkerError('Debug watch failed', error);
                await this.clearWatch(task);
            }
        } finally {
            task.compiling = false;
        }
    }

    private compile(
        snapshot: CompileSnapshot,
        profile: EmbeddedBuildProfile,
    ): Promise<EmbeddedCompileResponse> {
        const compiler = this.compiler;
        if (compiler === undefined) {
            return Promise.reject(new Error('the embedded compiler is not initialized'));
        }
        return compiler.compile({
            protocolVersion: compilerServiceProtocolVersion,
            uri: snapshot.key,
            sourcePath: snapshot.uri.scheme === 'file' ? snapshot.uri.fsPath : undefined,
            revision: snapshot.revision,
            source: snapshot.source,
            profile,
        }).then(response => {
            if (response.uri !== snapshot.key || response.revision !== snapshot.revision) {
                throw new Error(
                    `embedded compiler returned ${response.uri}@${response.revision} for ${snapshot.key}@${snapshot.revision}`,
                );
            }
            return response;
        });
    }

    private snapshot(document: vscode.TextDocument): CompileSnapshot {
        return {
            uri: document.uri,
            key: document.uri.toString(),
            name: resourceName(document.uri),
            revision: document.version,
            source: document.getText(),
        };
    }

    private async savedActiveScript(): Promise<vscode.TextDocument | undefined> {
        let result;
        try {
            result = await resolveSavedScript<vscode.Uri, vscode.TextDocument>({
                activeDocument: () => vscode.window.activeTextEditor?.document,
                save: uri => vscode.workspace.save(uri),
                saveAs: uri => vscode.workspace.saveAs(uri),
                openDocument: uri => vscode.workspace.openTextDocument(uri),
            });
        } catch (error) {
            this.reportWorkerError('The SplitScript file could not be saved', error);
            return undefined;
        }
        if ('failure' in result) {
            void vscode.window.showErrorMessage(savedScriptFailureMessage(result.failure));
            return undefined;
        }
        return result.document;
    }

    private beginOutput(
        profile: EmbeddedBuildProfile,
        snapshot: CompileSnapshot,
        output: vscode.Uri,
    ): void {
        this.outputChannel.clear();
        this.outputChannel.appendLine(
            `Embedded ${profile} build: ${snapshot.uri.toString(true)}`,
        );
        this.outputChannel.appendLine(`Output: ${output.toString(true)}`);
        this.outputChannel.appendLine('');
    }

    private appendDiagnostics(
        snapshot: CompileSnapshot,
        diagnostics: readonly EmbeddedServiceDiagnostic[],
    ): void {
        for (const diagnostic of diagnostics) {
            this.outputChannel.appendLine(
                `${snapshot.name}: ${diagnostic.severity}[${diagnostic.code}]: ${diagnostic.message}`,
            );
            for (const note of diagnostic.notes) {
                this.outputChannel.appendLine(`  = note: ${note}`);
            }
        }
    }

    private async replaceOutput(output: vscode.Uri, artifact: Uint8Array): Promise<void> {
        const temporary = output.with({
            path: `${output.path}.tmp-${Date.now()}-${this.temporaryCounter++}`,
            query: '',
            fragment: '',
        });
        await vscode.workspace.fs.writeFile(temporary, artifact);
        try {
            await vscode.workspace.fs.rename(temporary, output, { overwrite: true });
        } catch (error) {
            try {
                await vscode.workspace.fs.delete(temporary);
            } catch {
                // Preserve the original write/rename failure.
            }
            throw error;
        }
    }

    private reportWorkerError(context: string, error: unknown): void {
        this.outputChannel.appendLine(`${context}: ${errorMessage(error)}`);
        this.outputChannel.show(true);
        void vscode.window.showErrorMessage(`${context}: ${errorMessage(error)}`);
    }

    private async clearWatch(task: WatchTask): Promise<void> {
        if (!this.tasks.finish(task)) {
            return;
        }
        this.watchStatus.hide();
        await this.setWatchContext(false);
    }

    private setWatchContext(active: boolean): Thenable<unknown> {
        return vscode.commands.executeCommand('setContext', 'splitScript.watchActive', active);
    }
}

function resourceName(uri: vscode.Uri): string {
    const separator = uri.path.lastIndexOf('/');
    return uri.path.slice(separator + 1) || 'autosplitter.split';
}

function outputUri(input: vscode.Uri): vscode.Uri {
    const separator = input.path.lastIndexOf('/');
    const directory = input.path.slice(0, separator + 1);
    const name = input.path.slice(separator + 1);
    const dot = name.lastIndexOf('.');
    const stem = dot > 0 ? name.slice(0, dot) : name;
    return input.with({
        path: `${directory}${stem}.wasm`,
        query: '',
        fragment: '',
    });
}

function compilerErrorCode(error: unknown): string | undefined {
    return typeof error === 'object'
        && error !== null
        && 'code' in error
        && typeof error.code === 'string'
        ? error.code
        : undefined;
}

function savedScriptFailureMessage(failure: SavedScriptFailure): string {
    switch (failure) {
        case 'noScript':
            return 'Open a SplitScript file first.';
        case 'saveFailed':
            return 'The SplitScript file could not be saved.';
        case 'notSaved':
            return 'Save the SplitScript file first.';
        case 'wrongLanguage':
            return 'The saved file is not recognized as SplitScript.';
        case 'wrongResource':
            return 'The saved SplitScript document could not be resolved.';
    }
}
