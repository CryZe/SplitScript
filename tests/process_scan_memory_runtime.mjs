import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/process_scan_memory_runtime.mjs <autosplitter.wasm>");
}

const decoder = new TextDecoder();
const ranges = [
    { address: 0x1000n, size: 0x100n, flags: 2n },
    { address: 0x2000n, size: 0x100n, flags: 0n },
    { address: 0x100000n, size: 0x20000n, flags: 2n },
];
let instance;
let reads = [];
let bytesRead = 0;
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
    process_get_memory_range_count: () => BigInt(ranges.length),
    process_get_memory_range_address(_process, index) {
        return ranges[Number(index)].address;
    },
    process_get_memory_range_size(_process, index) {
        return ranges[Number(index)].size;
    },
    process_get_memory_range_flags(_process, index) {
        return ranges[Number(index)].flags;
    },
    process_read(_process, address, destination, size) {
        if (address === 0x2000n) {
            throw new Error("an unreadable memory range was scanned");
        }
        reads.push(address);
        bytesRead += size;
        const output = new Uint8Array(instance.exports.memory.buffer, destination, size);
        output.fill(0);
        if (address === 0x1000n) {
            output.set([0xde, 0xad, 0x42, 0xef], 7);
        }
        return 1;
    },
    runtime_print_message(pointer, length) {
        messages.push(text(pointer, length));
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
const polls = [];
for (let poll = 0; poll < 4; poll += 1) {
    reads = [];
    bytesRead = 0;
    instance.exports.update();
    polls.push({ reads: reads.map(Number), bytesRead, messages: [...messages] });
    if (bytesRead > 70 * 1024) {
        throw new Error(`scan poll exceeded its memory-read budget: ${bytesRead}`);
    }
}

if (polls.slice(0, 3).some((poll) => poll.messages.length !== 0)) {
    throw new Error(`scan completed before visiting the lower range: ${JSON.stringify(polls)}`);
}
if (polls[0].reads[0] !== 0x100000 || polls[1].reads[0] !== 0x110000) {
    throw new Error(`large range cursor was not preserved: ${JSON.stringify(polls)}`);
}
if (polls[2].reads.length !== 0) {
    throw new Error(`unreadable range was not skipped cooperatively: ${JSON.stringify(polls)}`);
}
if (JSON.stringify(messages) !== JSON.stringify(["4103"]) || polls[3].reads[0] !== 0x1000) {
    throw new Error(`completed scan differed: ${JSON.stringify(polls)}`);
}

console.log(JSON.stringify({ messages, polls }));
