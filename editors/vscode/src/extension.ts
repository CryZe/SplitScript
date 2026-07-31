import * as vscode from 'vscode';
import { CompilerTaskController } from './compilerTasks';
import { EmbeddedCompilerWorkerClient } from './embeddedCompilerWorkerClient';
import { ExtensionRuntime } from './extensionRuntime';
import { LanguageClientController } from './languageClient';

let runtime: ExtensionRuntime | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
    const compilerTasks = new CompilerTaskController(
        context,
        moduleBytes => EmbeddedCompilerWorkerClient.create(
            context.asAbsolutePath('dist/embeddedCompilerNodeWorker.js'),
            moduleBytes,
        ),
    );
    runtime = new ExtensionRuntime(compilerTasks, new LanguageClientController(context));
    await runtime.activate(context);
}

export async function deactivate(): Promise<void> {
    const active = runtime;
    runtime = undefined;
    await active?.deactivate();
}
