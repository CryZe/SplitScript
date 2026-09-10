import * as vscode from 'vscode';

import type { RuntimeSnapshot } from './runtimeProtocol';

export class StatisticsViewProvider implements
    vscode.TreeDataProvider<vscode.TreeItem>,
    vscode.Disposable
{
    private readonly changes = new vscode.EventEmitter<vscode.TreeItem | undefined>();
    private snapshot: RuntimeSnapshot | undefined;

    public readonly onDidChangeTreeData = this.changes.event;

    public update(snapshot: RuntimeSnapshot | undefined): void {
        this.snapshot = snapshot;
        this.changes.fire(undefined);
    }

    public getTreeItem(element: vscode.TreeItem): vscode.TreeItem {
        return element;
    }

    public getChildren(): vscode.TreeItem[] {
        const snapshot = this.snapshot;
        if (snapshot === undefined) return [];
        const handles = snapshot.settings.handleCount + snapshot.processes.length;
        return [
            item('Tick Rate', `${formatNumber(snapshot.tickRateHz)} Hz`),
            item(
                'Average Tick',
                `${formatNumber(snapshot.averageTickMilliseconds)} ms`,
                'Average update duration across recent retained samples.',
                'pulse',
            ),
            item(
                'Slowest Tick',
                `${formatNumber(snapshot.slowestTickMilliseconds)} ms`,
                'Slowest update since statistics were last reset.',
            ),
            item(
                'Handles',
                handles.toLocaleString(),
                'Current process, settings-map, settings-list, and setting-value handles.',
                'references',
            ),
            item(
                'Wasm Memory',
                formatBytes(snapshot.memoryBytes),
                'Current linear memory used by the debugged WebAssembly module, excluding its code.',
                'database',
                'wasmMemory',
            ),
        ];
    }

    public dispose(): void {
        this.changes.dispose();
    }
}

function item(
    label: string,
    description: string,
    tooltip?: string,
    icon?: string,
    contextValue?: string,
): vscode.TreeItem {
    const value = new vscode.TreeItem(label, vscode.TreeItemCollapsibleState.None);
    value.description = description;
    value.tooltip = tooltip ?? `${label}: ${description}`;
    value.contextValue = contextValue;
    if (icon !== undefined) value.iconPath = new vscode.ThemeIcon(icon);
    return value;
}

function formatNumber(value: number): string {
    if (value === 0) return '0';
    if (value < 0.01) return value.toFixed(4);
    return value.toFixed(value < 10 ? 3 : 1);
}

function formatBytes(bytes: number): string {
    if (bytes < 1_024) return `${bytes} B`;
    if (bytes < 1_024 * 1_024) return `${(bytes / 1_024).toFixed(1)} KiB`;
    return `${(bytes / (1_024 * 1_024)).toFixed(1)} MiB`;
}
