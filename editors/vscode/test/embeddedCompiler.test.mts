import assert from 'node:assert/strict';
import test from 'node:test';
import {
    EmbeddedCompilerProtocolError,
    decodeEmbeddedCompileResponse,
} from '../src/embeddedCompiler.ts';

function envelope(metadata: object, artifact: Uint8Array = new Uint8Array()): Uint8Array {
    const metadataBytes = new TextEncoder().encode(JSON.stringify(metadata));
    const response = new Uint8Array(8 + metadataBytes.length + artifact.length);
    response.set([0x53, 0x53, 0x43, 0x52]);
    new DataView(response.buffer).setUint32(4, metadataBytes.length, true);
    response.set(metadataBytes, 8);
    response.set(artifact, 8 + metadataBytes.length);
    return response;
}

test('decodes metadata and raw artifact bytes without base64', () => {
    const artifact = new Uint8Array([0, 0x61, 0x73, 0x6d]);
    const response = decodeEmbeddedCompileResponse(envelope({
        protocolVersion: 1,
        uri: 'file:///example.split',
        revision: 7,
        diagnostics: [],
        artifactLength: artifact.length,
        error: null,
    }, artifact));
    assert.equal(response.revision, 7);
    assert.deepEqual(response.artifact, artifact);
});

test('rejects a truncated response envelope', () => {
    assert.throws(
        () => decodeEmbeddedCompileResponse(new Uint8Array([0x53, 0x53])),
        (error: unknown) => error instanceof EmbeddedCompilerProtocolError
            && error.code === 'invalidResponse',
    );
});

test('surfaces a structured compiler-service error', () => {
    assert.throws(
        () => decodeEmbeddedCompileResponse(envelope({
            protocolVersion: 1,
            uri: '',
            revision: 0,
            diagnostics: [],
            artifactLength: 0,
            error: {
                code: 'unsupportedProtocol',
                message: 'wrong protocol',
            },
        })),
        (error: unknown) => error instanceof EmbeddedCompilerProtocolError
            && error.code === 'unsupportedProtocol'
            && error.message === 'wrong protocol',
    );
});
