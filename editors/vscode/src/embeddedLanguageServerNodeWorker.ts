import { parentPort, workerData } from 'node:worker_threads';
import type { Message } from 'vscode-jsonrpc';
import { EmbeddedLanguageServer } from './embeddedLanguageServer';

interface LanguageServerWorkerData {
    moduleBytes: ArrayBuffer;
}

interface ReadyMessage {
    kind: 'ready';
}

const port = requiredParentPort();
void start();

async function start(): Promise<void> {
    const data = workerData as LanguageServerWorkerData;
    const server = await EmbeddedLanguageServer.create(new Uint8Array(data.moduleBytes));

    port.on('message', (message: Message) => {
        for (const outgoing of server.handle(message)) {
            port.postMessage(outgoing);
        }
    });
    port.postMessage({ kind: 'ready' } satisfies ReadyMessage);
}

function requiredParentPort(): NonNullable<typeof parentPort> {
    if (parentPort === null) {
        throw new Error('the embedded language server must run in a worker thread');
    }
    return parentPort;
}
