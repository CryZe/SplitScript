import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/a_hat_in_time_runtime.mjs <autosplitter.wasm>");
}

const decoder = new TextDecoder();
const bytes = new Map();
const reads = [];
let instance;

const timerBase = 0x1100n;
const saveSignature = 0x1200n;
const actorsSignature = 0x1300n;
const coordinatesSignature = 0x1400n;
const saveRoot = 0x3000n;
const actorsRoot = 0x3100n;

const writeBytes = (address, values) => {
    values.forEach((value, index) => bytes.set(address + BigInt(index), value));
};
const writeNumber = (address, size, write) => {
    const buffer = new ArrayBuffer(size);
    write(new DataView(buffer));
    writeBytes(address, new Uint8Array(buffer));
};
const writeI32 = (address, value) => writeNumber(
    address,
    4,
    view => view.setInt32(0, value, true),
);
const writeU32 = (address, value) => writeNumber(
    address,
    4,
    view => view.setUint32(0, value, true),
);
const writeU64 = (address, value) => writeNumber(
    address,
    8,
    view => view.setBigUint64(0, value, true),
);
const writeF32 = (address, value) => writeNumber(
    address,
    4,
    view => view.setFloat32(0, value, true),
);
const writeF64 = (address, value) => writeNumber(
    address,
    8,
    view => view.setFloat64(0, value, true),
);
const writeRelative32 = (instructionAddress, target) => {
    writeI32(instructionAddress, Number(target - instructionAddress - 4n));
};

writeBytes(timerBase, [0x54, 0x49, 0x4d, 0x52]);
writeI32(timerBase + 0x04n, 1);
writeF64(timerBase + 0x08n, 2.5);
writeI32(timerBase + 0x10n, 0);
writeI32(timerBase + 0x14n, 1);
writeI32(timerBase + 0x18n, 1);
writeI32(timerBase + 0x1cn, 0);
writeI32(timerBase + 0x20n, 0);
writeF64(timerBase + 0x24n, 12.25);
writeF64(timerBase + 0x2cn, 11.5);
writeF64(timerBase + 0x34n, 13.25);
writeF64(timerBase + 0x3cn, 12.5);
writeI32(timerBase + 0x44n, 7);
writeBytes(timerBase + 0x48n, [0x45, 0x4e, 0x44, 0x20]);

writeBytes(saveSignature, [
    0x48, 0x8b, 0x05, 0, 0, 0, 0, 0x48, 0x8b, 0xd9, 0x48, 0x85, 0xc0, 0x75,
    0, 0x48, 0x89, 0x7c, 0x24, 0,
]);
writeRelative32(saveSignature + 3n, saveRoot);
writeBytes(actorsSignature, [
    0x48, 0x8b, 0x05, 0, 0, 0, 0, 0x81, 0x88, 0, 0, 0, 0, 0, 0, 0x80,
    0,
]);
writeRelative32(actorsSignature + 3n, actorsRoot);
writeBytes(coordinatesSignature, [
    0x48, 0x8b, 0x81, 0, 0, 0, 0, 0x4c, 0x8d, 0x80, 0, 0, 0, 0,
]);
writeU32(coordinatesSignature + 3n, 0x20);

// The Modding save layout selected by the third signature candidate.
writeU64(saveRoot, 1n);
writeU64(saveRoot + 0x64n, 0x4000n);
writeI32(0x4000n + 0xe0n, 23);
writeI32(0x4000n + 0xf8n, 4);
writeI32(0x4000n + 0xfcn, 2);
writeI32(0x4000n + 0x100n, 9);

// The actors path reaches Hat Kid and then its three coordinate fields.
writeU64(actorsRoot, 1n);
writeU64(actorsRoot + 0x6dcn, 0x5000n);
writeU64(0x5000n, 0x5100n);
writeU64(0x5100n + 0x68n, 0x5200n);
writeU64(0x5200n + 0x20n, 0x5300n);
writeF32(0x5300n + 0x80n, 100.5);
writeF32(0x5300n + 0x84n, -200.25);
writeF32(0x5300n + 0x88n, 300.75);

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 0,
    timer_start() {},
    timer_split() {},
    timer_reset() {},
    timer_pause_game_time() {},
    timer_resume_game_time() {},
    process_attach(pointer, length) {
        return text(pointer, length) === "HatinTimeGame" ? 1n : 0n;
    },
    process_detach() {},
    process_is_open: () => 1,
    process_get_memory_range_count: () => 1n,
    process_get_memory_range_address: () => 0x1000n,
    process_get_memory_range_size: () => 0x1000n,
    process_get_memory_range_flags: () => 2n,
    process_read(_process, address, destination, size) {
        reads.push([address, size]);
        const output = new Uint8Array(instance.exports.memory.buffer, destination, size);
        for (let index = 0; index < size; index += 1) {
            output[index] = bytes.get(address + BigInt(index)) ?? 0;
        }
        return 1;
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();

for (let poll = 0; poll < 20; poll += 1) {
    instance.exports.update();
    if (reads.some(([address]) => address === 0x40e0n)) {
        break;
    }
}

const expectedStateReads = [
    [timerBase + 0x04n, 4],
    [timerBase + 0x08n, 8],
    [timerBase + 0x44n, 4],
    [saveRoot + 0x64n, 8],
    [0x4000n + 0xe0n, 4],
    [0x4000n + 0xf8n, 4],
    [0x4000n + 0xfcn, 4],
    [0x4000n + 0x100n, 4],
    [actorsRoot + 0x6dcn, 8],
    [0x5300n + 0x80n, 4],
    [0x5300n + 0x84n, 4],
    [0x5300n + 0x88n, 4],
];
for (const expected of expectedStateReads) {
    if (!reads.some(([address, size]) => address === expected[0] && size === expected[1])) {
        throw new Error(
            `missing discovered state read ${expected[0].toString(16)}:${expected[1]} from ${JSON.stringify(
                reads.map(([address, size]) => `${address.toString(16)}:${size}`),
            )}`,
        );
    }
}
if (reads.some(([, size]) => size > 0x10000)) {
    throw new Error("a discovery poll attempted an unbounded memory read");
}

console.log(JSON.stringify({ pollsCompleted: true, reads: reads.length }));
