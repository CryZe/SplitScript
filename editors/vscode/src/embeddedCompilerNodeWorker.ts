import { parentPort } from 'node:worker_threads';
import {
    EmbeddedCompiler,
    EmbeddedCompilerProtocolError,
} from './embeddedCompiler';
import type {
    EmbeddedCompilerWorkerRequest,
    EmbeddedCompilerWorkerResponse,
} from './embeddedCompilerWorkerProtocol';

const port = requiredParentPort();

let compiler: EmbeddedCompiler | undefined;

port.on('message', (message: EmbeddedCompilerWorkerRequest) => {
    void handle(message);
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
    transfer: readonly ArrayBuffer[] = [],
): void {
    port.postMessage(response, [...transfer]);
}

function requiredParentPort(): NonNullable<typeof parentPort> {
    if (parentPort === null) {
        throw new Error('the embedded compiler worker must run in a worker thread');
    }
    return parentPort;
}
