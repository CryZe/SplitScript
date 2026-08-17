import assert from "node:assert/strict";
import { once } from "node:events";
import fs from "node:fs";
import { resolve } from "node:path";
import { Worker } from "node:worker_threads";

const workerPath = process.argv[2];
const modulePath = process.argv[3];
if (!workerPath || !modulePath) {
    throw new Error(
        "usage: node tests/embedded_language_server_worker_runtime.mjs <worker.js> <compiler.wasm>",
    );
}

const moduleBytes = fs.readFileSync(resolve(modulePath));
const ownedModuleBytes = new Uint8Array(moduleBytes.length);
ownedModuleBytes.set(moduleBytes);
const worker = new Worker(resolve(workerPath), {
    workerData: { moduleBytes: ownedModuleBytes.buffer },
    transferList: [ownedModuleBytes.buffer],
});

try {
    const [ready] = await once(worker, "message");
    assert.deepEqual(ready, { kind: "ready" });

    worker.postMessage({
        jsonrpc: "2.0",
        id: 1,
        method: "initialize",
        params: {},
    });
    const [initialized] = await once(worker, "message");
    assert.equal(initialized.id, 1);
    assert.equal(initialized.result.serverInfo.name, "splitls");
    assert.equal(initialized.result.capabilities.hoverProvider, true);

    worker.postMessage({
        jsonrpc: "2.0",
        id: 2,
        method: "splitscript/documentation/page",
        params: { uri: "/stdlib/types/Duration/index.md" },
    });
    const [documentation] = await once(worker, "message");
    assert.equal(documentation.id, 2);
    assert.equal(documentation.result.title, "Duration");
    assert.match(documentation.result.markdown, /\[fromSeconds\]\(methods\/fromSeconds\.md\)/);

    worker.postMessage({
        jsonrpc: "2.0",
        method: "textDocument/didOpen",
        params: {
            textDocument: {
                uri: "file:///worker-test.split",
                languageId: "splitscript",
                version: 4,
                text: 'state "game.exe" {',
            },
        },
    });
    const [diagnostics] = await once(worker, "message");
    assert.equal(diagnostics.method, "textDocument/publishDiagnostics");
    assert.equal(diagnostics.params.version, 4);
    assert.equal(diagnostics.params.diagnostics[0].code, "SS0002");

    worker.postMessage({
        jsonrpc: "2.0",
        id: 3,
        method: "shutdown",
        params: null,
    });
    const [shutdown] = await once(worker, "message");
    assert.equal(shutdown.id, 3);
    assert.equal(shutdown.result, null);
    worker.postMessage({ jsonrpc: "2.0", method: "exit", params: null });
} finally {
    await worker.terminate();
}

console.log("embedded language-server worker passed");
