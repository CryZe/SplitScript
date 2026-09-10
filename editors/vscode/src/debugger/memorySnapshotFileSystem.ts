import * as vscode from 'vscode';

export const MEMORY_SNAPSHOT_SCHEME = 'splitscript-memory';
export const HEX_EDITOR_VIEW_TYPE = 'hexEditor.hexedit';

interface MemorySnapshot {
    bytes: Uint8Array;
    created: number;
    opened: boolean;
}

export class MemorySnapshotFileSystem implements vscode.FileSystemProvider, vscode.Disposable {
    private readonly changes = new vscode.EventEmitter<vscode.FileChangeEvent[]>();
    private readonly snapshots = new Map<string, MemorySnapshot>();
    private nextId = 1;

    public readonly onDidChangeFile = this.changes.event;

    public create(name: string, bytes: Uint8Array): vscode.Uri {
        const uri = vscode.Uri.from({
            scheme: MEMORY_SNAPSHOT_SCHEME,
            path: `/${this.nextId++}-${safeName(name)}`,
        });
        this.snapshots.set(uri.toString(), {
            bytes,
            created: Date.now(),
            opened: false,
        });
        this.changes.fire([{ type: vscode.FileChangeType.Created, uri }]);
        return uri;
    }

    public markOpened(uri: vscode.Uri): void {
        const snapshot = this.snapshots.get(uri.toString());
        if (snapshot !== undefined) snapshot.opened = true;
    }

    public retainOpen(uris: readonly vscode.Uri[]): void {
        const retained = new Set(uris.map(uri => uri.toString()));
        for (const [key, snapshot] of this.snapshots) {
            if (snapshot.opened && !retained.has(key)) {
                this.snapshots.delete(key);
            }
        }
    }

    public remove(uri: vscode.Uri): void {
        if (this.snapshots.delete(uri.toString())) {
            this.changes.fire([{ type: vscode.FileChangeType.Deleted, uri }]);
        }
    }

    public watch(): vscode.Disposable {
        return new vscode.Disposable(() => {});
    }

    public stat(uri: vscode.Uri): vscode.FileStat {
        const snapshot = this.snapshot(uri);
        return {
            type: vscode.FileType.File,
            ctime: snapshot.created,
            mtime: snapshot.created,
            size: snapshot.bytes.byteLength,
            permissions: vscode.FilePermission.Readonly,
        };
    }

    public readDirectory(): [string, vscode.FileType][] {
        throw vscode.FileSystemError.FileNotADirectory();
    }

    public createDirectory(): void {
        throw vscode.FileSystemError.NoPermissions('Memory snapshots are read-only.');
    }

    public readFile(uri: vscode.Uri): Uint8Array {
        return this.snapshot(uri).bytes.slice();
    }

    public writeFile(): void {
        throw vscode.FileSystemError.NoPermissions('Memory snapshots are read-only.');
    }

    public delete(): void {
        throw vscode.FileSystemError.NoPermissions('Memory snapshots are read-only.');
    }

    public rename(): void {
        throw vscode.FileSystemError.NoPermissions('Memory snapshots are read-only.');
    }

    public dispose(): void {
        this.snapshots.clear();
        this.changes.dispose();
    }

    private snapshot(uri: vscode.Uri): MemorySnapshot {
        const snapshot = this.snapshots.get(uri.toString());
        if (snapshot === undefined) throw vscode.FileSystemError.FileNotFound(uri);
        return snapshot;
    }
}

export function openMemorySnapshotUris(): vscode.Uri[] {
    const uris: vscode.Uri[] = [];
    for (const group of vscode.window.tabGroups.all) {
        for (const tab of group.tabs) {
            const input = tab.input;
            if (
                input instanceof vscode.TabInputCustom
                && input.uri.scheme === MEMORY_SNAPSHOT_SCHEME
            ) {
                uris.push(input.uri);
            }
        }
    }
    return uris;
}

function safeName(name: string): string {
    const normalized = name.replace(/[^A-Za-z0-9._-]+/g, '-').replace(/^-+|-+$/g, '');
    return normalized.length === 0 ? 'memory.bin' : normalized;
}
