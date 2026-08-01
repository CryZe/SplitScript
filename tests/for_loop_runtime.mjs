import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/for_loop_runtime.mjs <loop.wasm>");
}

const decoder = new TextDecoder();
const messages = [];
let observed;
let instance;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 0,
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    runtime_print_message(pointer, length) {
        messages.push(text(pointer, length));
    },
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        if (text(keyPointer, keyLength) === "For Loop" && observed === undefined) {
            observed = text(valuePointer, valueLength);
        }
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();

for (let tick = 0; tick < 20 && !messages.includes("async done"); tick += 1) {
    instance.exports.update();
}
instance.exports.update();

const expectedMessages = ["first", "last", "async done"];
if (JSON.stringify(messages) !== JSON.stringify(expectedMessages) || observed !== "8,1") {
    throw new Error(`unexpected for-loop output: ${JSON.stringify({ expectedMessages, messages, observed })}`);
}

console.log(JSON.stringify({ messages, observed }));
