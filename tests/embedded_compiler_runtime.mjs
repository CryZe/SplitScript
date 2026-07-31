import assert from "node:assert/strict";
import fs from "node:fs";

const modulePath = process.argv[2];
if (!modulePath) {
    throw new Error("usage: node tests/embedded_compiler_runtime.mjs <compiler.wasm>");
}

const { instance } = await WebAssembly.instantiate(fs.readFileSync(modulePath), {});
const api = instance.exports;
assert.equal(api.splitscript_service_protocol_version(), 1);

function compile(source, revision) {
    const request = new TextEncoder().encode(JSON.stringify({
        protocolVersion: 1,
        uri: "file:///embedded-test.split",
        revision,
        source,
        profile: "release",
    }));
    const pointer = api.splitscript_service_alloc(request.length);
    try {
        new Uint8Array(api.memory.buffer, pointer, request.length).set(request);
        api.splitscript_service_compile(pointer, request.length);
    } finally {
        api.splitscript_service_dealloc(pointer, request.length);
    }

    const response = new Uint8Array(
        api.memory.buffer,
        api.splitscript_service_response_pointer(),
        api.splitscript_service_response_length(),
    ).slice();
    assert.deepEqual(response.subarray(0, 4), new Uint8Array([0x53, 0x53, 0x43, 0x52]));
    const metadataLength = new DataView(response.buffer).getUint32(4, true);
    const artifactOffset = 8 + metadataLength;
    const metadata = JSON.parse(new TextDecoder().decode(response.subarray(8, artifactOffset)));
    const artifact = response.subarray(artifactOffset);
    assert.equal(artifact.length, metadata.artifactLength);
    return { metadata, artifact };
}

const successful = compile('state "game.exe" {}', 11);
assert.equal(successful.metadata.revision, 11);
assert.equal(successful.metadata.error, null);
assert.deepEqual(successful.artifact.subarray(0, 4), new Uint8Array([0, 0x61, 0x73, 0x6d]));

const failed = compile("fn broken( {", 12);
assert.equal(failed.metadata.revision, 12);
assert.equal(failed.artifact.length, 0);
assert.ok(failed.metadata.diagnostics.length > 0);

console.log("embedded compiler WebAssembly protocol passed");
