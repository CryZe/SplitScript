import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import { resolve } from "node:path";

const [extensionPath, compilerWorkerPath, languageWorkerPath, modulePath] =
    process.argv.slice(2).map(path => path && resolve(path));
if (!extensionPath || !compilerWorkerPath || !languageWorkerPath || !modulePath) {
    throw new Error(
        "usage: node tests/web_worker_bundles_runtime.mjs <extension.js> <compiler-worker.js> <language-worker.js> <compiler.wasm>",
    );
}

const extension = fs.readFileSync(extensionPath, "utf8");
const externalRequires = new Set(
    [...extension.matchAll(/\brequire\("([^"]+)"\)/g)].map(match => match[1]),
);
assert.deepEqual([...externalRequires], ["vscode"]);

const moduleBytes = fs.readFileSync(modulePath);
const compiler = workerRuntime(compilerWorkerPath);
compiler.dispatch({
    id: 1,
    kind: "initialize",
    moduleBytes: copiedBuffer(moduleBytes),
});
const compilerReady = await compiler.nextMessage();
assert.equal(compilerReady.id, 1);
assert.equal(compilerReady.ok, true);
compiler.dispatch({
    id: 2,
    kind: "compile",
    request: {
        protocolVersion: 1,
        uri: "file:///browser-worker.split",
        revision: 9,
        source: 'state "game.exe" {}',
        profile: "release",
    },
});
const compilation = await compiler.nextMessage();
assert.equal(compilation.id, 2);
assert.equal(compilation.ok, true);
assert.equal(compilation.response.revision, 9);
assert.deepEqual(
    [...compilation.response.artifact.subarray(0, 4)],
    [0, 0x61, 0x73, 0x6d],
);
compiler.dispatch({
    id: 3,
    kind: "compile",
    request: {
        protocolVersion: 1,
        uri: "file:///browser-worker.split",
        revision: 10,
        source: 'state "game.exe" {}',
        profile: "debug",
    },
});
compiler.dispatch({ id: 4, kind: "cancel", targetId: 3 });
const cancellationAcknowledgement = await compiler.nextMessage();
assert.equal(cancellationAcknowledgement.id, 4);
assert.equal(cancellationAcknowledgement.ok, true);
const cancelledCompilation = await compiler.nextMessage();
assert.equal(cancelledCompilation.id, 3);
assert.equal(cancelledCompilation.ok, false);
assert.equal(cancelledCompilation.error.code, "cancelled");

const language = workerRuntime(languageWorkerPath);
language.dispatch({
    kind: "initialize",
    moduleBytes: copiedBuffer(moduleBytes),
});
assert.equal((await language.nextMessage()).kind, "ready");
language.dispatch({
    jsonrpc: "2.0",
    id: 3,
    method: "initialize",
    params: {},
});
const initialized = await language.nextMessage();
assert.equal(initialized.id, 3);
assert.equal(initialized.result.serverInfo.name, "splitls");
assert.equal(initialized.result.capabilities.semanticTokensProvider.full, true);
language.dispatch({
    jsonrpc: "2.0",
    id: 4,
    method: "splitscript/documentation/page",
    params: { uri: "/stdlib/types/Duration/index.md" },
});
const documentation = await language.nextMessage();
assert.equal(documentation.id, 4);
assert.equal(documentation.result.title, "Duration");
assert.match(documentation.result.markdown, /\[fromSeconds\]\(methods\/fromSeconds\.md\)/);

console.log("web extension and browser worker bundles passed");

function copiedBuffer(bytes) {
    const copy = new Uint8Array(bytes.length);
    copy.set(bytes);
    return copy.buffer;
}

function workerRuntime(scriptPath) {
    const listeners = new Map();
    const messages = [];
    const waiters = [];
    const scope = {
        addEventListener(type, listener) {
            const current = listeners.get(type) ?? [];
            current.push(listener);
            listeners.set(type, current);
        },
        removeEventListener(type, listener) {
            listeners.set(
                type,
                (listeners.get(type) ?? []).filter(candidate => candidate !== listener),
            );
        },
        postMessage(message) {
            const waiter = waiters.shift();
            if (waiter) {
                waiter(message);
            } else {
                messages.push(message);
            }
        },
    };
    vm.runInNewContext(fs.readFileSync(scriptPath, "utf8"), {
        self: scope,
        WebAssembly,
        TextEncoder,
        TextDecoder,
        Uint8Array,
        ArrayBuffer,
        DataView,
        console,
        setTimeout,
        clearTimeout,
    }, { filename: scriptPath });
    return {
        dispatch(data) {
            for (const listener of [...(listeners.get("message") ?? [])]) {
                listener({ data });
            }
        },
        nextMessage() {
            if (messages.length > 0) {
                return Promise.resolve(messages.shift());
            }
            return new Promise(resolveMessage => waiters.push(resolveMessage));
        },
    };
}
