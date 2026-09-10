import * as vscode from 'vscode';

export const DEBUG_MEMORY_SCHEME = 'vscode-debug-memory';
export const HEX_EDITOR_VIEW_TYPE = 'hexEditor.hexedit';

export async function openDebugMemory(
    sessionId: string,
    memoryReference: string,
    displayName: string,
    baseAddress?: string,
): Promise<void> {
    const uri = vscode.Uri.from({
        scheme: DEBUG_MEMORY_SCHEME,
        authority: sessionId,
        path: `/${encodeURIComponent(memoryReference)}/${encodeURIComponent(displayName)}.bin`,
        query: baseAddress === undefined ? undefined : `baseAddress=${hexAddress(baseAddress)}`,
    });
    await vscode.commands.executeCommand(
        'vscode.openWith',
        uri,
        HEX_EDITOR_VIEW_TYPE,
        { preview: false },
    );
}

function hexAddress(address: string): string {
    return `0x${BigInt(address).toString(16)}`;
}
