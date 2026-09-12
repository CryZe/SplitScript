import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/process_find_memory_range_runtime.mjs <autosplitter.wasm>");
}

const decoder = new TextDecoder();
const ranges = Array.from({ length: 300 }, (_, index) => ({
    address: BigInt(0x1000 + index * 0x1000),
    size: 0x100n,
    flags: 6n,
}));
const queried = [];
const queriesPerPoll = [];
const messages = [];
let countQueries = 0;
let currentPollQueries;
let instance;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);
const range = (index) => ranges[Number(index)];
const noteRangeQuery = (index) => {
    const numericIndex = Number(index);
    queried.push(numericIndex);
    currentPollQueries.push(numericIndex);
};

const env = {
    timer_get_state: () => 0,
    runtime_set_tick_rate() {},
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    process_get_memory_range_count() {
        countQueries += 1;
        return BigInt(ranges.length);
    },
    process_get_memory_range_address(_process, index) {
        noteRangeQuery(index);
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
for (let poll = 0; poll < 6; poll += 1) {
    currentPollQueries = [];
    instance.exports.update();
    queriesPerPoll.push(currentPollQueries);
    if (currentPollQueries.length > 128) {
        throw new Error(`memory-range poll exceeded its metadata budget: ${JSON.stringify(queriesPerPoll)}`);
    }
    if (poll === 2) {
        ranges.push({ address: 0x3000n, size: 0x300n, flags: 6n });
    }
}

if (JSON.stringify(messages) !== JSON.stringify(["12288|768|true|true|false"]) 
    || JSON.stringify(queriesPerPoll.map((queries) => queries.length))
        !== JSON.stringify([128, 128, 44, 1, 0, 0])
    || queried[0] !== 299
    || queried[299] !== 0
    || queried[300] !== 300
    || countQueries !== 2) {
    throw new Error(`unexpected cooperative discovery: ${JSON.stringify({
        messages,
        queried,
        queriesPerPoll,
        countQueries,
    })}`);
}

console.log(JSON.stringify({ messages, queried, queriesPerPoll, countQueries }));
