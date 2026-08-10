import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/collection_iteration_runtime.mjs <iteration.wasm>");
}

const decoder = new TextDecoder();
const variables = new Map();
let instance;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 0,
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        variables.set(text(keyPointer, keyLength), text(valuePointer, valueLength));
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();

let traps = 0;
for (let update = 0; update < 6; update += 1) {
    try {
        instance.exports.update();
    } catch (error) {
        if (!(error instanceof WebAssembly.RuntimeError)) {
            throw error;
        }
        traps += 1;
    }
}

const observed = {
    traps,
    value: variables.get("Collection Iteration"),
};
const expected = {
    traps: 2,
    value: "1,2",
};
if (JSON.stringify(observed) !== JSON.stringify(expected)) {
    throw new Error(`unexpected iteration output: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify(observed));
