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
let activeCompileId: number | undefined;
const cancelledCompiles = new Set<number>();

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
        if (message.kind === 'cancel') {
            if (activeCompileId === message.targetId) {
                cancelledCompiles.add(message.targetId);
                compiler?.discardCompile();
            }
            send({ id: message.id, ok: true });
            return;
        }
        if (compiler === undefined) {
            throw new Error('the embedded compiler worker is not initialized');
        }
        if (activeCompileId !== undefined) {
            cancelledCompiles.add(activeCompileId);
            compiler.discardCompile();
        }
        activeCompileId = message.id;
        const response = await compileStaged(compiler, message.id, message.request);
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

async function compileStaged(
    compiler: EmbeddedCompiler,
    id: number,
    request: Parameters<EmbeddedCompiler['compile']>[0],
): Promise<ReturnType<EmbeddedCompiler['compile']>> {
    try {
        if (compiler.startCompile(request) === 'complete') {
            return compiler.readResponse();
        }
        await yieldToHost();
        throwIfCancelled(id);
        if (compiler.lowerCompile() === 'complete') {
            return compiler.readResponse();
        }
        await yieldToHost();
        throwIfCancelled(id);
        compiler.finishCompile();
        return compiler.readResponse();
    } finally {
        cancelledCompiles.delete(id);
        if (activeCompileId === id) {
            activeCompileId = undefined;
        }
    }
}

function throwIfCancelled(id: number): void {
    if (activeCompileId !== id || cancelledCompiles.has(id)) {
        throw new EmbeddedCompilerProtocolError(
            'cancelled',
            'embedded compilation was superseded by a newer source revision',
        );
    }
}

function yieldToHost(): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, 0));
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
