import * as vscode from 'vscode';

interface ProtocolPosition {
    line: number;
    character: number;
}

interface ProtocolTextEdit {
    range: { start: ProtocolPosition; end: ProtocolPosition };
    newText: string;
}

interface EditorConfigLayer {
    relativePath: string;
    source: string;
}

type SendRequest = (
    method: string,
    params: unknown,
    token: vscode.CancellationToken,
) => Promise<ProtocolTextEdit[] | null>;

/** Adds filesystem-independent formatter policy to the ordinary LSP request. */
export async function provideDocumentFormattingEdits(
    document: vscode.TextDocument,
    options: vscode.FormattingOptions,
    token: vscode.CancellationToken,
    sendRequest: SendRequest,
): Promise<vscode.TextEdit[] | null> {
    const configuration = vscode.workspace.getConfiguration(
        'splitscript.formatting',
        document.uri,
    );
    const useEditorConfig = configuration.get<boolean>('useEditorConfig', true);
    const editorConfig = useEditorConfig
        ? await collectEditorConfig(document.uri, token)
        : [];
    if (token.isCancellationRequested) {
        return null;
    }

    const configured = {
        documentLineEnding: document.eol === vscode.EndOfLine.CRLF ? 'crlf' : 'lf',
        filesInsertFinalNewline: vscode.workspace
            .getConfiguration('files', document.uri)
            .get<boolean>('insertFinalNewline', false),
        maxLineWidth: optionalSetting<number>(configuration, 'maxLineWidth'),
        indentStyle: optionalSetting<string>(configuration, 'indentStyle'),
        indentWidth: optionalSetting<number>(configuration, 'indentWidth'),
        lineEnding: optionalSetting<string>(configuration, 'lineEnding'),
        insertFinalNewline: optionalSetting<boolean>(configuration, 'insertFinalNewline'),
        editorConfig,
    };
    const result = await sendRequest(
        'textDocument/formatting',
        {
            textDocument: { uri: document.uri.toString() },
            options: {
                tabSize: options.tabSize,
                insertSpaces: options.insertSpaces,
                splitscript: configured,
            },
        },
        token,
    );
    return result?.map((edit) => new vscode.TextEdit(
        new vscode.Range(
            edit.range.start.line,
            edit.range.start.character,
            edit.range.end.line,
            edit.range.end.character,
        ),
        edit.newText,
    )) ?? null;
}

function optionalSetting<T>(
    configuration: vscode.WorkspaceConfiguration,
    name: string,
): T | undefined {
    return configuration.get<T | null>(name) ?? undefined;
}

async function collectEditorConfig(
    target: vscode.Uri,
    token: vscode.CancellationToken,
): Promise<EditorConfigLayer[]> {
    if (target.scheme === 'untitled') {
        return [];
    }
    const layers: EditorConfigLayer[] = [];
    let directoryPath = parentPath(target.path);
    while (!token.isCancellationRequested) {
        const configUri = target.with({ path: joinPath(directoryPath, '.editorconfig') });
        try {
            const bytes = await vscode.workspace.fs.readFile(configUri);
            layers.push({
                relativePath: relativePath(directoryPath, target.path),
                source: new TextDecoder().decode(bytes),
            });
        } catch {
            // Missing and inaccessible ancestor files have the same effect.
        }
        const parent = parentPath(directoryPath);
        if (parent === directoryPath) {
            break;
        }
        directoryPath = parent;
    }
    return layers;
}

function parentPath(path: string): string {
    const normalized = path.replace(/\/+$/, '') || '/';
    const separator = normalized.lastIndexOf('/');
    return separator <= 0 ? '/' : normalized.slice(0, separator);
}

function joinPath(directory: string, name: string): string {
    return directory === '/' ? `/${name}` : `${directory}/${name}`;
}

function relativePath(directory: string, target: string): string {
    const prefix = directory === '/' ? '/' : `${directory}/`;
    return target.startsWith(prefix) ? target.slice(prefix.length) : target.split('/').at(-1) ?? target;
}
