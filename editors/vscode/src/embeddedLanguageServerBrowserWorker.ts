/// <reference lib="webworker" />

import type { Message } from 'vscode-jsonrpc';
import { EmbeddedLanguageServer } from './embeddedLanguageServer';

interface InitializeMessage {
    kind: 'initialize';
    moduleBytes: ArrayBuffer;
}

const initialize = (event: MessageEvent<InitializeMessage>): void => {
    if (event.data.kind !== 'initialize') {
        return;
    }
    self.removeEventListener('message', initialize);
    void start(event.data.moduleBytes);
};
self.addEventListener('message', initialize);

async function start(moduleBytes: ArrayBuffer): Promise<void> {
    const server = await EmbeddedLanguageServer.create(new Uint8Array(moduleBytes));
    self.addEventListener('message', event => {
        for (const outgoing of server.handle(event.data as Message)) {
            self.postMessage(outgoing);
        }
    });
    self.postMessage({ kind: 'ready' });
}
