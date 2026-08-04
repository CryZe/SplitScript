import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/on_split_runtime.mjs <autosplitter.wasm>");
}

const decoder = new TextDecoder();
const messages = [];
let instance;

const env = {
    timer_get_state: () => 0,
    process_attach: () => 0n,
    process_detach() {},
    process_is_open: () => 0,
    runtime_print_message(pointer, length) {
        messages.push(decoder.decode(
            new Uint8Array(instance.exports.memory.buffer, pointer, length),
        ));
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();

// Timer events are host-driven and can arrive without an attached process.
instance.exports.on_split();
instance.exports.on_split();

if (JSON.stringify(messages) !== JSON.stringify(["1", "2"])) {
    throw new Error(`unexpected onSplit observations: ${JSON.stringify(messages)}`);
}

console.log(JSON.stringify({ messages }));
