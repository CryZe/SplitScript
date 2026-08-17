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
let activeCompileId: number | undefined;
const cancelledCompiles = new Set<number>();

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
    transfer: readonly Transferable[] = [],
): void {
    self.postMessage(response, [...transfer]);
}
