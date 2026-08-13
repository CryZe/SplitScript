import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/set_runtime.mjs <set.wasm>");
}

const decoder = new TextDecoder();
const messages = [];
const variables = new Map();
let instance;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 0,
    runtime_set_tick_rate() {},
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    runtime_print_message(pointer, length) {
        messages.push(text(pointer, length));
    },
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        variables.set(text(keyPointer, keyLength), text(valuePointer, valueLength));
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
instance.exports.update();
instance.exports.update();
instance.exports.update();

const observed = {
    first: variables.get("Set First Tick"),
    second: variables.get("Set Second Tick"),
    messages,
};
const expected = {
    first: "true,false,5,true",
    second: "5,true,false,0,true",
    messages: ["A", "B", "D", "E"],
};
if (JSON.stringify(observed) !== JSON.stringify(expected)) {
    throw new Error(`unexpected set output: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify(observed));
