import * as vscode from 'vscode';
import { CompilerTaskController } from './compilerTasks';
import { EmbeddedCompilerWorkerClient } from './embeddedCompilerWorkerClient';
import { SplitScriptDebuggerController } from './debugger/debuggerController';
import { ExtensionRuntime } from './extensionRuntime';
import { LanguageClientController } from './languageClient';

let runtime: ExtensionRuntime | undefined;
let debuggerController: SplitScriptDebuggerController | undefined;

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
    debuggerController = new SplitScriptDebuggerController(
        context,
        moduleBytes => EmbeddedCompilerWorkerClient.create(
            context.asAbsolutePath('dist/embeddedCompilerNodeWorker.js'),
            moduleBytes,
        ),
    );
    await debuggerController.initialize();
}

export async function deactivate(): Promise<void> {
    debuggerController?.dispose();
    debuggerController = undefined;
    const active = runtime;
    runtime = undefined;
    await active?.deactivate();
}
