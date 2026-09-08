import * as vscode from 'vscode';

const MESSAGE = 'Running SplitScript autosplitters is available in desktop VS Code with a trusted local workspace.';

export function registerUnavailableDebugger(context: vscode.ExtensionContext): void {
    const unavailable = async () => {
        await vscode.window.showErrorMessage(MESSAGE);
    };
    const provider: vscode.DebugConfigurationProvider = {
        resolveDebugConfiguration: () => {
            void unavailable();
            return undefined;
        },
    };
    const emptyTree: vscode.TreeDataProvider<vscode.TreeItem> = {
        getTreeItem: item => item,
        getChildren: () => [],
    };
    context.subscriptions.push(
        vscode.debug.registerDebugConfigurationProvider('splitscript', provider),
        ...[
            'splitscript.debugger.runtime',
            'splitscript.debugger.settings',
            'splitscript.debugger.settingsMap',
            'splitscript.debugger.variables',
            'splitscript.debugger.processes',
        ].map(view => vscode.window.registerTreeDataProvider(view, emptyTree)),
        ...[
            'splitscript.debug.start',
            'splitscript.debug.restart',
            'splitscript.debug.stop',
            'splitscript.debug.timerStart',
            'splitscript.debug.timerReset',
            'splitscript.debug.showLogs',
            'splitscript.debug.editSetting',
            'splitscript.debug.clearSettings',
        ].map(command => vscode.commands.registerCommand(command, unavailable)),
    );
}
