import * as vscode from 'vscode';
import { CompilerTaskController } from './compilerTasks';
import { EmbeddedCompilerBrowserWorkerClient } from './embeddedCompilerBrowserWorkerClient';
import { ExtensionRuntime } from './extensionRuntime';
import { BrowserLanguageClientController } from './languageClientBrowser';

let runtime: ExtensionRuntime | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
    const compilerWorker = vscode.Uri.joinPath(
        context.extensionUri,
        'dist',
        'web',
        'embeddedCompilerWorker.js',
    ).toString(true);
    const compilerTasks = new CompilerTaskController(
        context,
        moduleBytes => EmbeddedCompilerBrowserWorkerClient.create(
            compilerWorker,
            moduleBytes,
        ),
    );
    runtime = new ExtensionRuntime(
        compilerTasks,
        new BrowserLanguageClientController(context),
    );
    await runtime.activate(context);
}

export async function deactivate(): Promise<void> {
    const active = runtime;
    runtime = undefined;
    await active?.deactivate();
}
