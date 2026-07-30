import * as fs from 'node:fs';
import * as path from 'node:path';
import { ChildProcessWithoutNullStreams, spawn } from 'node:child_process';
import * as vscode from 'vscode';
import {
    Executable,
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;
let buildingRelease = false;
let watchSession: WatchSession | undefined;
let watchStatus: vscode.StatusBarItem | undefined;

interface WatchSession {
    child: ChildProcessWithoutNullStreams;
    input: string;
    output: string;
    stopping: boolean;
    errorReported: boolean;
}

export async function activate(context: vscode.ExtensionContext): Promise<void> {
    const compilerOutput = vscode.window.createOutputChannel('SplitScript Compiler');
    watchStatus = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
    watchStatus.command = 'splitscript.stopDebugWatch';
    context.subscriptions.push(
        compilerOutput,
        watchStatus,
        vscode.commands.registerCommand(
            'splitscript.restartLanguageServer',
            async () => restartClient(context),
        ),
        vscode.commands.registerCommand(
            'splitscript.buildRelease',
            async () => buildRelease(context, compilerOutput),
        ),
        vscode.commands.registerCommand(
            'splitscript.startDebugWatch',
            async () => startDebugWatch(context, compilerOutput),
        ),
        vscode.commands.registerCommand(
            'splitscript.stopDebugWatch',
            async () => stopDebugWatch(true),
        ),
        vscode.workspace.onDidChangeConfiguration(async event => {
            if (
                event.affectsConfiguration('splitScript.server.path')
                || event.affectsConfiguration('splitScript.server.arguments')
            ) {
                await restartClient(context);
            }
        }),
    );
    await vscode.commands.executeCommand('setContext', 'splitScript.watchActive', false);
    await startClient(context);
}

async function buildRelease(
    context: vscode.ExtensionContext,
    outputChannel: vscode.OutputChannel,
): Promise<void> {
    if (buildingRelease) {
        void vscode.window.showInformationMessage('A SplitScript release build is already running.');
        return;
    }
    if (watchSession !== undefined) {
        const action = await vscode.window.showWarningMessage(
            'Stop the debug watcher before building a release module.',
            'Stop Watch and Build Release',
        );
        if (action !== 'Stop Watch and Build Release') {
            return;
        }
        await stopDebugWatch(false);
    }
    const document = await savedActiveScript();
    if (document === undefined) {
        return;
    }

    const command = compilerExecutable(context, document.uri);
    const input = document.uri.fsPath;
    const output = path.join(path.dirname(input), `${path.parse(input).name}.wasm`);
    const args = [input, '--profile', 'release'];

    outputChannel.clear();
    outputChannel.appendLine(`> ${displayCommand(command, args)}`);
    outputChannel.appendLine('');
    buildingRelease = true;
    let exitCode: number;
    try {
        exitCode = await vscode.window.withProgress(
            {
                location: vscode.ProgressLocation.Notification,
                title: `Building ${path.basename(input)} for release`,
            },
            async () => runCompiler(command, args, path.dirname(input), outputChannel),
        );
    } catch (error) {
        buildingRelease = false;
        await showCompilerLaunchError(error, outputChannel);
        return;
    }
    // The child process, not any follow-up notification, owns the build lock.
    buildingRelease = false;

    if (exitCode === 0) {
        outputChannel.appendLine('');
        outputChannel.appendLine(`Created ${output}`);
        void vscode.window.showInformationMessage(
            `Built release module ${path.basename(output)}.`,
            'Reveal Output',
        ).then(async action => {
            if (action === 'Reveal Output') {
                await vscode.commands.executeCommand('revealFileInOS', vscode.Uri.file(output));
            }
        });
    } else {
        outputChannel.show(true);
        void vscode.window.showErrorMessage(
            `SplitScript release build failed with exit code ${exitCode}. See the compiler output for details.`,
        );
    }
}

async function startDebugWatch(
    context: vscode.ExtensionContext,
    outputChannel: vscode.OutputChannel,
): Promise<void> {
    if (buildingRelease) {
        void vscode.window.showInformationMessage(
            'Wait for the SplitScript release build to finish before starting debug watch.',
        );
        return;
    }
    const document = await savedActiveScript();
    if (document === undefined) {
        return;
    }
    if (watchSession !== undefined) {
        const watchedName = path.basename(watchSession.input);
        void vscode.window.showInformationMessage(
            watchSession.input === document.uri.fsPath
                ? `${watchedName} is already being watched.`
                : `Debug watch is already active for ${watchedName}. Stop it before watching another script.`,
        );
        return;
    }

    const command = compilerExecutable(context, document.uri);
    const input = document.uri.fsPath;
    const output = path.join(path.dirname(input), `${path.parse(input).name}.wasm`);
    const args = ['watch', input, '--profile', 'debug'];
    outputChannel.clear();
    outputChannel.appendLine(`> ${displayCommand(command, args)}`);
    outputChannel.appendLine('');

    const child = spawn(command, args, { cwd: path.dirname(input), windowsHide: true });
    const session: WatchSession = {
        child,
        input,
        output,
        stopping: false,
        errorReported: false,
    };
    watchSession = session;
    if (watchStatus !== undefined) {
        watchStatus.text = `$(sync~spin) Debug watch: ${path.basename(input)}`;
        watchStatus.tooltip = 'SplitScript debug watch is active. Click to stop.';
        watchStatus.show();
    }
    await vscode.commands.executeCommand('setContext', 'splitScript.watchActive', true);

    child.stdout.on('data', chunk => outputChannel.append(chunk.toString()));
    child.stderr.on('data', chunk => {
        outputChannel.append(chunk.toString());
        outputChannel.show(true);
    });
    child.once('error', error => {
        session.errorReported = true;
        void showCompilerLaunchError(error, outputChannel);
    });
    child.once('close', code => {
        outputChannel.appendLine('');
        outputChannel.appendLine(
            session.stopping
                ? 'Debug watch stopped.'
                : `Debug watch exited${code === null ? '' : ` with code ${code}`}.`,
        );
        void clearWatchSession(session);
        if (!session.stopping && !session.errorReported) {
            outputChannel.show(true);
            void vscode.window.showErrorMessage(
                `SplitScript debug watch stopped unexpectedly${code === null ? '.' : ` with exit code ${code}.`}`,
            );
        }
    });

    void vscode.window.showInformationMessage(
        `Debug watch started for ${path.basename(input)}. Saving the file recompiles ${path.basename(output)}.`,
    );
}

async function stopDebugWatch(showNotification: boolean): Promise<void> {
    const session = watchSession;
    if (session === undefined) {
        if (showNotification) {
            void vscode.window.showInformationMessage('No SplitScript debug watch is active.');
        }
        return;
    }
    session.stopping = true;
    const closed = new Promise<void>(resolve => session.child.once('close', () => resolve()));
    if (session.child.exitCode === null && session.child.signalCode === null) {
        session.child.kill();
        await Promise.race([closed, delay(2_000)]);
    }
    await clearWatchSession(session);
    if (showNotification) {
        void vscode.window.showInformationMessage(
            `Stopped debug watch for ${path.basename(session.input)}.`,
        );
    }
}

async function clearWatchSession(session: WatchSession): Promise<void> {
    if (watchSession !== session) {
        return;
    }
    watchSession = undefined;
    watchStatus?.hide();
    await vscode.commands.executeCommand('setContext', 'splitScript.watchActive', false);
}

async function savedActiveScript(): Promise<vscode.TextDocument | undefined> {
    const document = vscode.window.activeTextEditor?.document;
    if (document === undefined || document.languageId !== 'splitscript') {
        void vscode.window.showErrorMessage('Open a SplitScript file first.');
        return undefined;
    }
    if (document.uri.scheme !== 'file') {
        void vscode.window.showErrorMessage('Save the SplitScript file first.');
        return undefined;
    }
    if (document.isDirty && !await document.save()) {
        void vscode.window.showErrorMessage('The SplitScript file could not be saved.');
        return undefined;
    }
    return document;
}

async function showCompilerLaunchError(
    error: unknown,
    outputChannel: vscode.OutputChannel,
): Promise<void> {
    outputChannel.show(true);
    const action = await vscode.window.showErrorMessage(
        `Could not run splitc: ${errorMessage(error)}`,
        'Open Compiler Settings',
    );
    if (action === 'Open Compiler Settings') {
        await vscode.commands.executeCommand(
            'workbench.action.openSettings',
            'splitScript.compiler.path',
        );
    }
}

function delay(milliseconds: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, milliseconds));
}

function runCompiler(
    command: string,
    args: string[],
    cwd: string,
    outputChannel: vscode.OutputChannel,
): Promise<number> {
    return new Promise((resolve, reject) => {
        const child = spawn(command, args, { cwd, windowsHide: true });
        child.stdout.on('data', chunk => outputChannel.append(chunk.toString()));
        child.stderr.on('data', chunk => outputChannel.append(chunk.toString()));
        child.once('error', reject);
        child.once('close', code => resolve(code ?? 1));
    });
}

function compilerExecutable(context: vscode.ExtensionContext, document: vscode.Uri): string {
    const configuration = vscode.workspace.getConfiguration('splitScript', document);
    const configuredPath = configuration.get<string>('compiler.path', '').trim();
    const workspace = vscode.workspace.getWorkspaceFolder(document)?.uri.fsPath
        ?? vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    if (configuredPath !== '') {
        return resolveConfiguredPath(configuredPath, workspace);
    }

    const executable = process.platform === 'win32' ? 'splitc.exe' : 'splitc';
    const configuredServer = configuration.get<string>('server.path', '').trim();
    const candidates = [
        context.asAbsolutePath(path.join('server', executable)),
        ...(configuredServer === ''
            ? []
            : [path.join(path.dirname(resolveConfiguredPath(configuredServer, workspace)), executable)]),
        context.asAbsolutePath(path.join('..', '..', 'target', 'debug', executable)),
    ];
    return candidates.find(candidate => fs.existsSync(candidate)) ?? executable;
}

function displayCommand(command: string, args: string[]): string {
    return [command, ...args]
        .map(argument => /\s|"/.test(argument) ? JSON.stringify(argument) : argument)
        .join(' ');
}

export async function deactivate(): Promise<void> {
    await stopDebugWatch(false);
    await stopClient();
}

async function restartClient(context: vscode.ExtensionContext): Promise<void> {
    await stopClient();
    await startClient(context);
}

async function startClient(context: vscode.ExtensionContext): Promise<void> {
    const executable = serverExecutable(context);
    const serverOptions: ServerOptions = {
        run: executable,
        debug: executable,
    };
    const clientOptions: LanguageClientOptions = {
        documentSelector: [
            { scheme: 'file', language: 'splitscript' },
            { scheme: 'untitled', language: 'splitscript' },
        ],
    };
    client = new LanguageClient(
        'splitscript',
        'SplitScript Language Server',
        serverOptions,
        clientOptions,
    );
    try {
        await client.start();
    } catch (error) {
        client = undefined;
        const choice = await vscode.window.showErrorMessage(
            `Could not start splitls: ${errorMessage(error)}`,
            'Open Server Settings',
        );
        if (choice === 'Open Server Settings') {
            await vscode.commands.executeCommand(
                'workbench.action.openSettings',
                'splitScript.server.path',
            );
        }
    }
}

async function stopClient(): Promise<void> {
    const running = client;
    client = undefined;
    if (running !== undefined) {
        await running.stop();
    }
}

function serverExecutable(context: vscode.ExtensionContext): Executable {
    const configuration = vscode.workspace.getConfiguration('splitScript');
    const configuredPath = configuration.get<string>('server.path', '').trim();
    const args = configuration.get<string[]>('server.arguments', []);
    const workspace = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    const command = configuredPath === ''
        ? discoverServer(context)
        : resolveConfiguredPath(configuredPath, workspace);
    return {
        command,
        args,
        options: {
            cwd: workspace ?? context.extensionPath,
        },
    };
}

function discoverServer(context: vscode.ExtensionContext): string {
    const executable = process.platform === 'win32' ? 'splitls.exe' : 'splitls';
    const candidates = [
        context.asAbsolutePath(path.join('server', executable)),
        context.asAbsolutePath(path.join('..', '..', 'target', 'debug', executable)),
    ];
    return candidates.find(candidate => fs.existsSync(candidate)) ?? 'splitls';
}

function resolveConfiguredPath(configuredPath: string, workspace: string | undefined): string {
    if (path.isAbsolute(configuredPath) || workspace === undefined) {
        return configuredPath;
    }
    return path.resolve(workspace, configuredPath);
}

function errorMessage(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}
