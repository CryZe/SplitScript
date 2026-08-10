import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/array_push_runtime.mjs <array.wasm>");
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
instance.exports.update();
instance.exports.update();
instance.exports.update();

const observed = variables.get("Array Push");
const expected = "1,1,70,1,fresh";
if (observed !== expected) {
    throw new Error(`unexpected array output: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify({ value: observed }));
