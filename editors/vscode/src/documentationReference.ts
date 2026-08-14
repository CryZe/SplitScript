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
                async () => this.openPage(INDEX_URI),
            ),
            vscode.commands.registerCommand(
                'splitscript.searchDocumentation',
                async () => this.search(),
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
        const items: DocumentationQuickPickItem[] = [
            {
                label: '$(book) Standard library index',
                description: 'reference',
                detail: 'Browse every documented SplitScript standard-library symbol.',
                uri: INDEX_URI,
                alwaysShow: true,
            },
            ...entries.map(entry => ({
                label: entry.title,
                description: entry.kind,
                detail: entry.signature ?? entry.summary,
                uri: entry.uri,
            })),
        ];
        const selected = await vscode.window.showQuickPick(items, {
            title: 'SplitScript documentation',
            placeHolder: 'Search standard-library symbols',
            matchOnDescription: true,
            matchOnDetail: true,
        });
        return selected?.uri;
    }
}

function normalizePageUri(uri: string): string {
    return uri.startsWith('/') ? uri : `/${uri}`;
}
