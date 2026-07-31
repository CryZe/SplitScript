/// <reference lib="webworker" />

import {
    EmbeddedCompiler,
    EmbeddedCompilerProtocolError,
} from './embeddedCompiler';
import type {
    EmbeddedCompilerWorkerRequest,
    EmbeddedCompilerWorkerResponse,
} from './embeddedCompilerWorkerProtocol';

let compiler: EmbeddedCompiler | undefined;

self.addEventListener('message', event => {
    void handle(event.data as EmbeddedCompilerWorkerRequest);
});

async function handle(message: EmbeddedCompilerWorkerRequest): Promise<void> {
    try {
        if (message.kind === 'initialize') {
            compiler = await EmbeddedCompiler.create(new Uint8Array(message.moduleBytes));
            send({ id: message.id, ok: true });
            return;
        }
        if (compiler === undefined) {
            throw new Error('the embedded compiler worker is not initialized');
        }
        const response = compiler.compile(message.request);
        const transfer = response.artifact === undefined
            ? []
            : [response.artifact.buffer as ArrayBuffer];
        send({ id: message.id, ok: true, response }, transfer);
    } catch (error) {
        send({
            id: message.id,
            ok: false,
            error: {
                name: error instanceof Error ? error.name : 'Error',
                message: error instanceof Error ? error.message : String(error),
                ...(error instanceof EmbeddedCompilerProtocolError
                    ? { code: error.code }
                    : {}),
            },
        });
    }
}

function send(
    response: EmbeddedCompilerWorkerResponse,
    transfer: readonly Transferable[] = [],
): void {
    self.postMessage(response, [...transfer]);
}
