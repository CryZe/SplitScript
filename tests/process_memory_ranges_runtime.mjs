import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/process_memory_ranges_runtime.mjs <autosplitter.wasm>");
}

const decoder = new TextDecoder();
const ranges = [
    { address: 0x1000n, size: 0x100n, flags: 2n },
    { address: 0x2000n, size: 0x200n, flags: 6n },
    { address: 0x3000n, size: 0x300n, flags: 10n },
];
const queried = [];
const messages = [];
let instance;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const range = (index) => ranges[Number(index)];
const env = {
    timer_get_state: () => 0,
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    process_get_memory_range_count: () => BigInt(ranges.length),
    process_get_memory_range_address(_process, index) {
        queried.push(Number(index));
        return range(index).address;
    },
    process_get_memory_range_size: (_process, index) => range(index).size,
    process_get_memory_range_flags: (_process, index) => range(index).flags,
    runtime_print_message(pointer, length) {
        messages.push(text(pointer, length));
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
for (let update = 0; update < 8; update += 1) {
    instance.exports.update();
}

const expectedMessages = [
    "4096|256|true|false|false",
    "8192|512|true|true|false",
    "12288|768|true|false|true",
];
if (JSON.stringify(queried) !== JSON.stringify([0, 1, 2])
    || JSON.stringify(messages) !== JSON.stringify(expectedMessages)) {
    throw new Error(`unexpected memory-range snapshot: ${JSON.stringify({ queried, messages })}`);
}

console.log(JSON.stringify({ messages, queried }));
