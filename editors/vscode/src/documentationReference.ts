import * as vscode from 'vscode';

const DOCUMENTATION_SCHEME = 'splitscript-docs';
const INDEX_URI = '/index.md';

interface DocumentationClient {
    sendRequest<TResult>(method: string, params?: unknown): Promise<TResult>;
}

interface DocumentationIndexEntry {
    uri: string;
    title: string;
    kind: string;
    summary: string;
    signature?: string;
}

interface DocumentationPage {
    uri: string;
    title: string;
    markdown: string;
}

interface DocumentationQuickPickItem extends vscode.QuickPickItem {
    uri: string;
}

/**
 * Presents compiler-owned documentation through VS Code's built-in Markdown
 * preview. The extension owns only navigation and presentation; symbol data,
 * Markdown, links, and examples remain language-server responses.
 */
export class DocumentationReferenceController implements vscode.Disposable {
    private readonly disposables: vscode.Disposable[] = [];

    public constructor(private readonly client: DocumentationClient) {}

    public register(): void {
        this.disposables.push(
            vscode.workspace.registerTextDocumentContentProvider(
                DOCUMENTATION_SCHEME,
                {
                    provideTextDocumentContent: async uri => {
                        const page = await this.client.sendRequest<DocumentationPage>(
                            'splitscript/documentation/page',
                            { uri: uri.path },
                        );
                        return page.markdown;
                    },
                },
            ),
            vscode.commands.registerCommand(
                'splitscript.openDocumentation',
                async (uri?: unknown) => this.openPage(
                    typeof uri === 'string' ? uri : INDEX_URI,
                ),
            ),
            vscode.commands.registerCommand(
                'splitscript.searchDocumentation',
                async () => this.search(),
            ),
            vscode.commands.registerCommand(
                'splitscript.openSymbolDocumentation',
                async () => this.openCurrentSymbol(),
            ),
        );
    }

    public dispose(): void {
        for (const disposable of this.disposables.splice(0)) {
            disposable.dispose();
        }
    }

    private async search(): Promise<void> {
        const uri = await this.pickPage();
        if (uri === undefined) {
            return;
        }

        await this.openPage(uri);
    }

    private async openCurrentSymbol(): Promise<void> {
        const editor = vscode.window.activeTextEditor;
        if (editor === undefined || editor.document.languageId !== 'splitscript') {
            await vscode.window.showInformationMessage(
                'Open a SplitScript source file to look up documentation.',
            );
            return;
        }

        const definitions = await vscode.commands.executeCommand<
            readonly (vscode.Location | vscode.LocationLink)[]
        >(
            'vscode.executeDefinitionProvider',
            editor.document.uri,
            editor.selection.active,
        );
        const documentation = definitions
            ?.map(definitionUri)
            .find(uri => uri.scheme === DOCUMENTATION_SCHEME);
        if (documentation === undefined) {
            await vscode.window.showInformationMessage(
                'No SplitScript documentation is available here.',
            );
            return;
        }

        await this.openPage(documentation.path);
    }

    private async openPage(uri: string): Promise<void> {
        const documentUri = vscode.Uri.from({
            scheme: DOCUMENTATION_SCHEME,
            path: normalizePageUri(uri),
        });
        // Opening first asks the registered content provider for the page and
        // produces a useful language-server error before opening an empty
        // preview if an identity ever becomes stale.
        await vscode.workspace.openTextDocument(documentUri);
        await vscode.commands.executeCommand('markdown.showPreviewToSide', documentUri);
    }

    private async pickPage(): Promise<string | undefined> {
        const entries = await this.client.sendRequest<DocumentationIndexEntry[]>(
            'splitscript/documentation/index',
            {},
        );
        const picker = vscode.window.createQuickPick<DocumentationQuickPickItem>();
        picker.title = 'SplitScript documentation';
        picker.placeholder = 'Search language, migration, and standard-library documentation';
        picker.matchOnDescription = false;
        picker.matchOnDetail = false;
        picker.items = documentationItems(entries, true);

        let request = 0;
        const selection = new Promise<string | undefined>(resolve => {
            picker.onDidChangeValue(value => {
                const currentRequest = ++request;
                const query = value.trim();
                if (query.length === 0) {
                    picker.busy = false;
                    picker.items = documentationItems(entries, true);
                    return;
                }

                picker.busy = true;
                void this.client.sendRequest<DocumentationIndexEntry[]>(
                    'splitscript/documentation/search',
                    { query },
                ).then(results => {
                    if (currentRequest === request) {
                        picker.items = documentationItems(results, false);
                        picker.busy = false;
                    }
                }, async error => {
                    if (currentRequest === request) {
                        picker.busy = false;
                        await vscode.window.showErrorMessage(
                            `Could not search SplitScript documentation: ${String(error)}`,
                        );
                    }
                });
            });
            picker.onDidAccept(() => {
                resolve(picker.activeItems[0]?.uri);
                picker.hide();
            });
            picker.onDidHide(() => resolve(undefined));
        });
        picker.show();
        try {
            return await selection;
        } finally {
            picker.dispose();
        }
    }
}

function documentationItems(
    entries: readonly DocumentationIndexEntry[],
    includeIndex: boolean,
): DocumentationQuickPickItem[] {
    const items = entries.map(entry => ({
        label: entry.title,
        description: entry.kind,
        detail: entry.signature ?? entry.summary,
        uri: entry.uri,
        // The compiler already filtered and ranked these entries, so VS Code's
        // label-only fuzzy matcher must not hide foreign-spelling matches.
        alwaysShow: true,
    }));
    if (includeIndex) {
        items.unshift({
            label: '$(book) SplitScript reference',
            description: 'reference',
            detail: 'Browse the language, migration guidance, and standard library.',
            uri: INDEX_URI,
            alwaysShow: true,
        });
    }
    return items;
}

function normalizePageUri(uri: string): string {
    return uri.startsWith('/') ? uri : `/${uri}`;
}

function definitionUri(definition: vscode.Location | vscode.LocationLink): vscode.Uri {
    return 'targetUri' in definition ? definition.targetUri : definition.uri;
}
