import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/string_predicates_runtime.mjs <string-predicates.wasm>");
}

const decoder = new TextDecoder();
let instance;
let observed;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 0,
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        if (text(keyPointer, keyLength) === "String Predicates") {
            observed = text(valuePointer, valueLength);
        }
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
instance.exports.update(); // Initialize old and current.
instance.exports.update();

const expected = "2097151";
if (observed !== expected) {
    throw new Error(`unexpected string-predicate output: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify({ observed }));
