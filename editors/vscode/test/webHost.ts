import * as vscode from 'vscode';

const extensionId = 'livesplit.splitscript';

export async function run(): Promise<void> {
    const extension = vscode.extensions.getExtension(extensionId);
    assert(extension !== undefined, `extension ${extensionId} is not installed`);
    await extension.activate();

    const workspace = vscode.workspace.workspaceFolders?.[0];
    assert(workspace !== undefined, 'the web test has no virtual workspace');
    assert(
        workspace.uri.scheme === 'vscode-test-web',
        `expected a vscode-test-web workspace, got ${workspace.uri.scheme}`,
    );

    const script = vscode.Uri.joinPath(workspace.uri, 'web.split');
    const document = await vscode.workspace.openTextDocument(script);
    await vscode.window.showTextDocument(document);

    const hoverPosition = document.positionAt(document.getText().indexOf('double') + 1);
    await hoverAt(script, hoverPosition);

    await vscode.commands.executeCommand('splitscript.restartLanguageServer');
    await hoverAt(script, hoverPosition);

    const durationDocs = vscode.Uri.parse(
        'splitscript-docs:/stdlib/types/Duration.md',
    );
    const documentation = await vscode.workspace.openTextDocument(durationDocs);
    assert(documentation.languageId === 'markdown', 'virtual documentation is not Markdown');
    assert(
        documentation.getText().includes('# Duration')
            && documentation.getText().includes('Duration.fromSeconds'),
        'the bundled language server returned incomplete standard-library documentation',
    );

    const output = script.with({ path: script.path.replace(/\.split$/, '.wasm') });
    await vscode.commands.executeCommand('splitscript.buildRelease');
    const release = await readWasm(output);

    await vscode.commands.executeCommand('splitscript.startDebugWatch');
    try {
        const debug = await waitFor(async () => {
            const bytes = await tryRead(output);
            return bytes !== undefined && !bytesEqual(bytes, release) ? bytes : undefined;
        }, 'debug watch did not replace the release module');

        const closingBrace = document.getText().lastIndexOf('}');
        const edit = new vscode.WorkspaceEdit();
        edit.insert(
            script,
            document.positionAt(closingBrace),
            '    debug print("web watch rebuild")\n',
        );
        assert(await vscode.workspace.applyEdit(edit), 'could not edit the virtual document');
        assert(await document.save(), 'could not save the virtual document');

        await waitFor(async () => {
            const bytes = await tryRead(output);
            return bytes !== undefined && !bytesEqual(bytes, debug) ? true : undefined;
        }, 'saving did not rebuild the debug module');
    } finally {
        await vscode.commands.executeCommand('splitscript.stopDebugWatch');
    }
}

async function hoverAt(uri: vscode.Uri, position: vscode.Position): Promise<void> {
    await waitFor(async () => {
        const hovers = await vscode.commands.executeCommand<readonly vscode.Hover[]>(
            'vscode.executeHoverProvider',
            uri,
            position,
        );
        return hovers !== undefined && hovers.length > 0 ? true : undefined;
    }, 'the browser language server returned no hover');
}

async function readWasm(uri: vscode.Uri): Promise<Uint8Array> {
    const bytes = await waitFor(
        () => tryRead(uri),
        `compiler did not create ${uri.toString(true)}`,
    );
    assert(
        bytes.length >= 4
            && bytes[0] === 0
            && bytes[1] === 0x61
            && bytes[2] === 0x73
            && bytes[3] === 0x6d,
        'compiler output is not a WebAssembly module',
    );
    return bytes;
}

async function tryRead(uri: vscode.Uri): Promise<Uint8Array | undefined> {
    try {
        return await vscode.workspace.fs.readFile(uri);
    } catch {
        return undefined;
    }
}

async function waitFor<T>(
    query: () => Promise<T | undefined>,
    failure: string,
    timeoutMilliseconds = 30_000,
): Promise<T> {
    const deadline = Date.now() + timeoutMilliseconds;
    while (Date.now() < deadline) {
        const value = await query();
        if (value !== undefined) {
            return value;
        }
        await new Promise(resolve => setTimeout(resolve, 50));
    }
    throw new Error(failure);
}

function bytesEqual(left: Uint8Array, right: Uint8Array): boolean {
    return left.length === right.length && left.every((byte, index) => byte === right[index]);
}

function assert(condition: unknown, message: string): asserts condition {
    if (!condition) {
        throw new Error(message);
    }
}
