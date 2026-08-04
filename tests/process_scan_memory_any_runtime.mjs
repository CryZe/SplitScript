import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/process_scan_memory_any_runtime.mjs <autosplitter.wasm>");
}

const decoder = new TextDecoder();
let instance;
let reads = 0;
const readsPerPoll = [];
const messages = [];

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 0,
    process_attach(pointer, length) {
        return text(pointer, length) === "game.exe" ? 1n : 0n;
    },
    process_detach() {},
    process_is_open: () => 1,
    process_get_memory_range_count: () => 1n,
    process_get_memory_range_address: () => 0x1000n,
    process_get_memory_range_size: () => 0x100n,
    process_get_memory_range_flags: () => 2n,
    process_read(_process, address, destination, size) {
        reads += 1;
        const output = new Uint8Array(instance.exports.memory.buffer, destination, size);
        output.fill(0);
        if (address === 0x1000n) {
            output.set([0xde, 0xad, 0x42, 0xef], 9);
        }
        return 1;
    },
    runtime_print_message(pointer, length) {
        messages.push(text(pointer, length));
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
for (let poll = 0; poll < 3; poll += 1) {
    reads = 0;
    instance.exports.update();
    readsPerPoll.push(reads);
}

if (JSON.stringify(readsPerPoll) !== JSON.stringify([1, 1, 0])) {
    throw new Error(
        `expected one candidate signature per work poll and a delivery poll, got ${JSON.stringify(readsPerPoll)}`,
    );
}
if (JSON.stringify(messages) !== JSON.stringify(["1:4105"])) {
    throw new Error(`unexpected signature match: ${JSON.stringify(messages)}`);
}

console.log(JSON.stringify({ readsPerPoll, messages }));
