import * as path from 'node:path';

import * as vscode from 'vscode';

import type { RuntimeSnapshot } from './runtimeProtocol';

export class RuntimeViewProvider implements vscode.TreeDataProvider<vscode.TreeItem>, vscode.Disposable {
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
        if (snapshot === undefined) {
            return [];
        }
        return [
            item('Program', path.basename(snapshot.program), 'file-code'),
            item('Status', snapshot.status, snapshot.status === 'trapped' ? 'error' : 'pulse'),
            item('Timer State', words(snapshot.timer.state), 'watch'),
            item('Game Time', formatDuration(snapshot.timer.gameTimeSeconds)),
            item('Game Time State', words(snapshot.timer.gameTimeState)),
            item('Split Index', String(snapshot.timer.splitIndex)),
            item('Tick Rate', `${formatNumber(snapshot.tickRateHz)} Hz`),
            item('Ticks', String(snapshot.tickCount)),
            item('Average Tick', `${formatNumber(snapshot.averageTickMilliseconds)} ms`),
            item('Slowest Tick', `${formatNumber(snapshot.slowestTickMilliseconds)} ms`),
            item('Wasm Memory', formatBytes(snapshot.memoryBytes), 'database'),
            item('ASR Handles', String(snapshot.settings.handleCount), 'references'),
        ];
    }

    public dispose(): void {
        this.changes.dispose();
    }
}

function item(label: string, description: string, icon?: string): vscode.TreeItem {
    const value = new vscode.TreeItem(label, vscode.TreeItemCollapsibleState.None);
    value.description = description;
    value.tooltip = `${label}: ${description}`;
    if (icon !== undefined) {
        value.iconPath = new vscode.ThemeIcon(icon);
    }
    return value;
}

function formatDuration(seconds: number): string {
    const sign = seconds < 0 ? '-' : '';
    const absolute = Math.abs(seconds);
    const hours = Math.floor(absolute / 3_600);
    const minutes = Math.floor(absolute / 60) % 60;
    const remainder = absolute % 60;
    return `${sign}${hours}:${String(minutes).padStart(2, '0')}:${remainder.toFixed(3).padStart(6, '0')}`;
}

function formatNumber(value: number): string {
    return value === 0 ? '0' : value.toFixed(value < 10 ? 3 : 1);
}

function formatBytes(bytes: number): string {
    if (bytes < 1_024) {
        return `${bytes} B`;
    }
    if (bytes < 1_024 * 1_024) {
        return `${(bytes / 1_024).toFixed(1)} KiB`;
    }
    return `${(bytes / (1_024 * 1_024)).toFixed(1)} MiB`;
}

function words(value: string): string {
    return value.replace(/[A-Z]/g, letter => ` ${letter.toLowerCase()}`)
        .replace(/^./, first => first.toUpperCase());
}
