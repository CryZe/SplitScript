import * as vscode from 'vscode';

import type {
    RuntimeSnapshot,
    SettingValueSnapshot,
    SettingWidgetSnapshot,
} from './runtimeProtocol';

interface WidgetNode {
    kind: 'widget';
    widget: SettingWidgetSnapshot;
    children: WidgetNode[];
}

interface ValueNode {
    kind: 'value';
    label: string;
    value: SettingValueSnapshot;
}

interface PlainNode {
    kind: 'plain';
    item: vscode.TreeItem;
}

interface ProcessNode {
    kind: 'process';
    process: import('./runtimeProtocol').ProcessSnapshot;
}

type DebugTreeNode = WidgetNode | ValueNode | PlainNode | ProcessNode;

abstract class SnapshotTreeProvider implements
    vscode.TreeDataProvider<DebugTreeNode>,
    vscode.Disposable
{
    private readonly changes = new vscode.EventEmitter<DebugTreeNode | undefined>();
    protected snapshot: RuntimeSnapshot | undefined;

    public readonly onDidChangeTreeData = this.changes.event;

    public update(snapshot: RuntimeSnapshot | undefined): void {
        this.snapshot = snapshot;
        this.changes.fire(undefined);
    }

    public abstract getTreeItem(element: DebugTreeNode): vscode.TreeItem;
    public abstract getChildren(element?: DebugTreeNode): DebugTreeNode[];

    public dispose(): void {
        this.changes.dispose();
    }
}

export class SettingsViewProvider extends SnapshotTreeProvider {
    public widget(key: string): SettingWidgetSnapshot | undefined {
        return this.snapshot?.settings.widgets.find(widget => widget.key === key);
    }

    public value(key: string): boolean | string | undefined {
        const widget = this.widget(key);
        const stored = this.snapshot?.settings.map.find(entry => entry.key === key)?.value;
        if (widget?.type === 'bool') return stored?.type === 'bool' ? stored.value : widget.defaultValue;
        if (widget?.type === 'choice') {
            return stored?.type === 'string' ? stored.value : widget.defaultOptionKey;
        }
        if (widget?.type === 'textInput') {
            return stored?.type === 'string' ? stored.value : widget.defaultValue;
        }
        if (widget?.type === 'fileSelect') return stored?.type === 'string' ? stored.value : '';
        return undefined;
    }

    public getTreeItem(element: DebugTreeNode): vscode.TreeItem {
        if (element.kind !== 'widget') return element.kind === 'plain' ? element.item : new vscode.TreeItem('');
        const widget = element.widget;
        const hasChildren = element.children.length > 0;
        const item = new vscode.TreeItem(
            widget.description,
            hasChildren ? vscode.TreeItemCollapsibleState.Expanded : vscode.TreeItemCollapsibleState.None,
        );
        item.tooltip = widget.tooltip ?? `${widget.description} (${widget.key})`;
        if (widget.type === 'title') {
            item.iconPath = new vscode.ThemeIcon('symbol-namespace');
            return item;
        }
        const value = this.value(widget.key);
        if (widget.type === 'choice') {
            item.description = widget.options.find(option => option.key === value)?.description ?? String(value ?? '');
            item.iconPath = new vscode.ThemeIcon('list-selection');
        } else if (widget.type === 'bool') {
            item.description = value ? 'Enabled' : 'Disabled';
            item.iconPath = new vscode.ThemeIcon(value ? 'check' : 'circle-large-outline');
        } else if (widget.type === 'fileSelect') {
            item.description = value === '' ? 'Not selected' : String(value);
            item.iconPath = new vscode.ThemeIcon('file');
        } else {
            item.description = String(value ?? '');
            item.iconPath = new vscode.ThemeIcon('edit');
        }
        item.command = {
            command: 'splitscript.debug.editSetting',
            title: 'Edit Setting',
            arguments: [widget.key],
        };
        return item;
    }

    public getChildren(element?: DebugTreeNode): DebugTreeNode[] {
        if (element?.kind === 'widget') return element.children;
        if (element !== undefined) return [];
        return settingHierarchy(this.snapshot?.settings.widgets ?? []);
    }
}

export class SettingsMapViewProvider extends SnapshotTreeProvider {
    public getTreeItem(element: DebugTreeNode): vscode.TreeItem {
        if (element.kind !== 'value') return element.kind === 'plain' ? element.item : new vscode.TreeItem('');
        const item = new vscode.TreeItem(
            element.label,
            isContainer(element.value)
                ? vscode.TreeItemCollapsibleState.Collapsed
                : vscode.TreeItemCollapsibleState.None,
        );
        item.description = describeValue(element.value);
        item.tooltip = `${element.label}: ${item.description}`;
        item.iconPath = new vscode.ThemeIcon(isContainer(element.value) ? 'json' : valueIcon(element.value));
        return item;
    }

    public getChildren(element?: DebugTreeNode): DebugTreeNode[] {
        if (element === undefined) {
            return (this.snapshot?.settings.map ?? []).map(entry => ({
                kind: 'value', label: entry.key, value: entry.value,
            }));
        }
        if (element.kind !== 'value') return [];
        if (element.value.type === 'map') {
            return element.value.value.map(entry => ({
                kind: 'value', label: entry.key, value: entry.value,
            }));
        }
        if (element.value.type === 'list') {
            return element.value.value.map((value, index) => ({
                kind: 'value', label: `[${index}]`, value,
            }));
        }
        return [];
    }
}

export class VariablesViewProvider extends SnapshotTreeProvider {
    public getTreeItem(element: DebugTreeNode): vscode.TreeItem {
        return element.kind === 'plain' ? element.item : new vscode.TreeItem('');
    }

    public getChildren(element?: DebugTreeNode): DebugTreeNode[] {
        if (element !== undefined) return [];
        return Object.entries(this.snapshot?.timer.variables ?? {}).map(([name, value]) => {
            const item = new vscode.TreeItem(name, vscode.TreeItemCollapsibleState.None);
            item.description = value;
            item.tooltip = `${name}: ${value}`;
            item.iconPath = new vscode.ThemeIcon('symbol-variable');
            return { kind: 'plain', item };
        });
    }
}

export class ProcessesViewProvider extends SnapshotTreeProvider {
    public getTreeItem(element: DebugTreeNode): vscode.TreeItem {
        if (element.kind === 'plain') return element.item;
        if (element.kind !== 'process') return new vscode.TreeItem('');
        const process = element.process;
        const name = process.path?.split(/[\\/]/).at(-1) ?? `PID ${process.pid}`;
        const item = new vscode.TreeItem(name, vscode.TreeItemCollapsibleState.Collapsed);
        item.description = `PID ${process.pid} · ${process.isOpen ? 'Open' : 'Closed'}`;
        item.tooltip = process.path ?? `PID ${process.pid}`;
        item.iconPath = new vscode.ThemeIcon(process.isOpen ? 'server-process' : 'circle-slash');
        return item;
    }

    public getChildren(element?: DebugTreeNode): DebugTreeNode[] {
        if (element === undefined) {
            return (this.snapshot?.processes ?? []).map(process => ({ kind: 'process', process }));
        }
        if (element.kind !== 'process') return [];
        return [
            plainItem('PID', String(element.process.pid), 'symbol-number'),
            plainItem('Handle', element.process.handle, 'references'),
            plainItem('State', element.process.isOpen ? 'Open' : 'Closed'),
            plainItem('Path', element.process.path ?? 'Unavailable', 'file-binary'),
        ];
    }
}

function settingHierarchy(widgets: readonly SettingWidgetSnapshot[]): WidgetNode[] {
    const roots: WidgetNode[] = [];
    const stack: Array<{ level: number; children: WidgetNode[] }> = [{ level: -1, children: roots }];
    for (const widget of widgets) {
        if (widget.type === 'title') {
            while (stack.length > 1 && stack.at(-1)!.level >= widget.headingLevel) stack.pop();
            const node: WidgetNode = { kind: 'widget', widget, children: [] };
            stack.at(-1)!.children.push(node);
            stack.push({ level: widget.headingLevel, children: node.children });
        } else {
            stack.at(-1)!.children.push({ kind: 'widget', widget, children: [] });
        }
    }
    return roots;
}

function isContainer(value: SettingValueSnapshot): boolean {
    return value.type === 'map' || value.type === 'list';
}

function describeValue(value: SettingValueSnapshot): string {
    if (value.type === 'map') return `${value.value.length} entries`;
    if (value.type === 'list') return `${value.value.length} items`;
    return String(value.value);
}

function valueIcon(value: SettingValueSnapshot): string {
    if (value.type === 'bool') return value.value ? 'check' : 'circle-large-outline';
    if (value.type === 'string') return 'symbol-string';
    return 'symbol-number';
}

function plainItem(label: string, description: string, icon?: string): PlainNode {
    const item = new vscode.TreeItem(label, vscode.TreeItemCollapsibleState.None);
    item.description = description;
    item.tooltip = `${label}: ${description}`;
    if (icon !== undefined) item.iconPath = new vscode.ThemeIcon(icon);
    return { kind: 'plain', item };
}
