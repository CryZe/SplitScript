import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/async_loop_runtime.mjs <loop.wasm>");
}

const decoder = new TextDecoder();
const messages = [];
let readPolls = 0;
let processOpen = true;
let nextHandle = 1n;
let instance;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 0,
    runtime_set_tick_rate() {},
    process_attach: () => processOpen ? nextHandle++ : 0n,
    process_detach() {},
    process_is_open: () => processOpen ? 1 : 0,
    process_read(_process, address, destination, size) {
        if (address !== 0x3000n || size !== 4) return 0;
        readPolls += 1;
        if (readPolls % 2 === 1) return 0;
        new DataView(instance.exports.memory.buffer).setInt32(destination, 42, true);
        return 1;
    },
    runtime_print_message(pointer, length) {
        messages.push(text(pointer, length));
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();

// Suspend during the second iteration, close the process, and verify that a
// fresh attachment restarts both the async loop state and its live counters.
instance.exports.update();
instance.exports.update();
processOpen = false;
instance.exports.update();
processOpen = true;

for (let tick = 0; tick < 30 && !messages.includes("done"); tick += 1) {
    instance.exports.update();
}

const expected = [
    "value 1",
    "value 1",
    "value 3",
    "retry 0:42",
    "retry 1:42",
    "nested 1:1",
    "nested 2:1",
    "compound 7:1:1:1",
    "done",
];
if (JSON.stringify(messages) !== JSON.stringify(expected)) {
    throw new Error(`unexpected async-loop output: ${JSON.stringify({ expected, messages })}`);
}

console.log(JSON.stringify({ messages, readPolls }));
