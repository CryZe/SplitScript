import assert from "node:assert/strict";
import fs from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";

const workerPath = process.argv[2];
const modulePath = process.argv[3];
if (!workerPath || !modulePath) {
    throw new Error(
        "usage: node tests/embedded_compiler_worker_runtime.mjs <worker.js> <compiler.wasm>",
    );
}

const require = createRequire(import.meta.url);
const { EmbeddedCompilerWorkerClient } = require(
    "../editors/vscode/dist/embeddedCompilerWorkerClient.js",
);
const client = await EmbeddedCompilerWorkerClient.create(
    resolve(workerPath),
    fs.readFileSync(resolve(modulePath)),
);

try {
    const successful = await client.compile({
        protocolVersion: 1,
        uri: "file:///worker-test.split",
        revision: 21,
        source: 'state "game.exe" {}',
        profile: "release",
    });
    assert.equal(successful.revision, 21);
    assert.deepEqual(
        successful.artifact.subarray(0, 4),
        new Uint8Array([0, 0x61, 0x73, 0x6d]),
    );

    const failed = await client.compile({
        protocolVersion: 1,
        uri: "file:///worker-test.split",
        revision: 22,
        source: "fn broken( {",
        profile: "debug",
    });
    assert.equal(failed.revision, 22);
    assert.equal(failed.artifact, undefined);
    assert.ok(failed.diagnostics.length > 0);
} finally {
    client.dispose();
}

console.log("embedded compiler worker passed");
