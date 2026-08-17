import assert from 'node:assert/strict';
import test from 'node:test';
import {
    EmbeddedCompilerWorkerConnection,
    type EmbeddedCompilerWorkerPort,
} from '../src/embeddedCompilerWorkerConnection.ts';
import type {
    EmbeddedCompilerWorkerRequest,
    EmbeddedCompilerWorkerResponse,
} from '../src/embeddedCompilerWorkerProtocol.ts';

class FakeWorkerPort implements EmbeddedCompilerWorkerPort {
    public requests: EmbeddedCompilerWorkerRequest[] = [];
    public transfers: readonly (readonly ArrayBuffer[])[] = [];
    public terminated = false;
    private messageListener: ((message: EmbeddedCompilerWorkerResponse) => void) | undefined;
    private failureListener: ((error: Error) => void) | undefined;

    public onMessage(listener: (message: EmbeddedCompilerWorkerResponse) => void): void {
        this.messageListener = listener;
    }

    public onFailure(listener: (error: Error) => void): void {
        this.failureListener = listener;
    }

    public postMessage(
        message: EmbeddedCompilerWorkerRequest,
        transfer: readonly ArrayBuffer[],
    ): void {
        this.requests.push(message);
        this.transfers = [...this.transfers, transfer];
        if (message.kind === 'initialize') {
            queueMicrotask(() => this.messageListener?.({ id: message.id, ok: true }));
        }
    }

    public terminate(): void {
        this.terminated = true;
    }

    public fail(error: Error): void {
        this.failureListener?.(error);
    }

    public respond(message: EmbeddedCompilerWorkerResponse): void {
        this.messageListener?.(message);
    }
}

test('shared worker connection initializes with a transferred module buffer', async () => {
    const port = new FakeWorkerPort();
    const connection = await EmbeddedCompilerWorkerConnection.create(
        port,
        new Uint8Array([0, 1, 2]),
    );
    assert.equal(port.requests[0]?.kind, 'initialize');
    assert.equal(port.transfers[0]?.length, 1);
    connection.dispose();
    assert.equal(port.terminated, true);
});

test('shared worker connection rejects pending work after either host fails', async () => {
    const port = new FakeWorkerPort();
    const connection = await EmbeddedCompilerWorkerConnection.create(
        port,
        new Uint8Array([0]),
    );
    const compilation = connection.compile({
        protocolVersion: 1,
        uri: 'file:///failure.split',
        revision: 2,
        source: 'state "game.exe" {}',
        profile: 'debug',
    });
    port.fail(new Error('worker failed'));
    await assert.rejects(compilation, /worker failed/);
    connection.dispose();
});

test('cancels the active staged compilation with a typed outcome', async () => {
    const port = new FakeWorkerPort();
    const connection = await EmbeddedCompilerWorkerConnection.create(
        port,
        new Uint8Array([0]),
    );
    const compilation = connection.compile({
        protocolVersion: 1,
        uri: 'file:///superseded.split',
        revision: 3,
        source: 'state "game.exe" {}',
        profile: 'debug',
    });
    const compile = port.requests.at(-1);
    assert.equal(compile?.kind, 'compile');
    connection.cancelCurrentCompilation();
    const cancel = port.requests.at(-1);
    assert.equal(cancel?.kind, 'cancel');
    if (compile?.kind !== 'compile' || cancel?.kind !== 'cancel') {
        throw new Error('expected compile and cancel requests');
    }
    assert.equal(cancel.targetId, compile.id);
    port.respond({ id: cancel.id, ok: true });
    port.respond({
        id: compile.id,
        ok: false,
        error: {
            name: 'EmbeddedCompilerProtocolError',
            message: 'superseded',
            code: 'cancelled',
        },
    });
    await assert.rejects(
        compilation,
        (error: unknown) => typeof error === 'object'
            && error !== null
            && 'code' in error
            && error.code === 'cancelled',
    );
    connection.dispose();
});
