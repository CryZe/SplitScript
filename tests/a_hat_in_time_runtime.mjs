import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/a_hat_in_time_runtime.mjs <autosplitter.wasm>");
}

const decoder = new TextDecoder();
const bytes = new Map();
const reads = [];
const readsPerPoll = [];
const settingValues = new Map();
const settingValueHandles = new Map();
const registeredBooleanKeys = new Set();
let nextSettingValueHandle = 1n;
let instance;

const expectedSettingKeys = new Set([
    "manySplits",
    "manySplits_1_1_cp1",
    "manySplits_1_2_cp1",
    "manySplits_1_3_cp1",
    "manySplits_1_4_cp0_pause_pos",
    "manySplits_1_4_cp1",
    "manySplits_1_4_cp2",
    "manySplits_1_6_cp59_pause",
    "manySplits_2_1_cp5",
    "manySplits_2_4_cp1",
    "manySplits_2_4_cp4",
    "manySplits_2_5_cp1",
    "manySplits_2_5_cp2",
    "manySplits_2_5_cp3",
    "manySplits_2_6_cp0_pause_pos",
    "manySplits_3_2_cp0_pause_pos",
    "manySplits_3_3_cp1",
    "manySplits_3_4_cp0_pause_pos",
    "manySplits_3_6_cp1",
    "manySplits_4_99_cp0_pause_pos",
    "manySplits_4_99_entry",
    "manySplits_5_1_cp1",
    "manySplits_5_1_cp10",
    "manySplits_5_1_cp2",
    "manySplits_5_1_cp3",
    "manySplits_6_1_cp3",
    "manySplits_6_2_cp1",
    "manySplits_6_3_cp1",
    "manySplits_6_3_cp2",
    "manySplits_6_3_cp3",
    "manySplits_pos_40",
    "manySplits_pos_41",
    "manySplits_pos_42",
    "manySplits_pos_43",
    "manySplits_pos_44",
    "manySplits_pos_45",
    "manySplits_pos_5",
    "manySplits_riftBlue",
    "manySplits_riftPurple",
    "settings",
    "settings_gameTimeMsg",
    "settings_ILMode",
    "settings_newFileStart",
    "splits",
    "splits_actEntry",
    "splits_checkpoint",
    "splits_dwbth",
    "splits_dwbth_doubleSplitNo",
    "splits_tp",
    "splits_tp_any",
    "splits_tp_new",
    "splits_tp_std",
    "splits_yarn",
]);
for (let chapter = 1; chapter <= 7; chapter += 1) {
    expectedSettingKeys.add(`manySplits_${chapter}`);
}
for (const [chapter, actCount] of [[1, 7], [2, 6], [3, 6], [6, 3]]) {
    for (let act = 1; act <= actCount; act += 1) {
        const base = `manySplits_${chapter}_${act}`;
        expectedSettingKeys.add(base);
        expectedSettingKeys.add(`${base}_entry`);
        expectedSettingKeys.add(`${base}_tp`);
        expectedSettingKeys.add(`${base}_tpDelayed`);
    }
}
for (const chapter of [5, 7]) {
    expectedSettingKeys.add(`manySplits_${chapter}_entry`);
}
for (const chapter of [4, 5, 7]) {
    expectedSettingKeys.add(`manySplits_${chapter}_tp`);
}
for (const id of [
    "gallery", "lab", "sewers", "bazaar", "owlExpress", "moon",
    "village", "pipe", "twilight", "curly", "balcony",
]) {
    const base = `manySplits_riftBlue_${id}`;
    expectedSettingKeys.add(base);
    expectedSettingKeys.add(`${base}_entry`);
    expectedSettingKeys.add(`${base}_tp`);
}
for (const id of ["moc", "dbs", "sleepy", "alpine", "deepSea", "rumbi", "tour"]) {
    const base = `manySplits_riftPurple_${id}`;
    expectedSettingKeys.add(base);
    expectedSettingKeys.add(`${base}_entry`);
    expectedSettingKeys.add(`${base}_cp`);
    expectedSettingKeys.add(`${base}_tp`);
}

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
    user_settings_add_bool(keyPointer, keyLength, _labelPointer, _labelLength, defaultValue) {
        const key = text(keyPointer, keyLength);
        registeredBooleanKeys.add(key);
        if (!settingValues.has(key)) {
            settingValues.set(key, defaultValue !== 0);
        }
        return settingValues.get(key) ? 1 : 0;
    },
    user_settings_add_title() {},
    user_settings_set_tooltip() {},
    settings_map_load: () => 1n,
    settings_map_free() {},
    settings_map_get(_map, keyPointer, keyLength) {
        const key = text(keyPointer, keyLength);
        if (!settingValues.has(key)) {
            return 0n;
        }
        const handle = nextSettingValueHandle++;
        settingValueHandles.set(handle, settingValues.get(key));
        return handle;
    },
    setting_value_free(handle) {
        settingValueHandles.delete(handle);
    },
    setting_value_get_bool(handle, outputPointer) {
        const value = settingValueHandles.get(handle);
        if (typeof value !== "boolean") {
            return 0;
        }
        new DataView(instance.exports.memory.buffer).setUint8(outputPointer, value ? 1 : 0);
        return 1;
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();

const missingSettingKeys = [...expectedSettingKeys]
    .filter(key => !registeredBooleanKeys.has(key));
const unexpectedSettingKeys = [...registeredBooleanKeys]
    .filter(key => !expectedSettingKeys.has(key));
if (missingSettingKeys.length !== 0 || unexpectedSettingKeys.length !== 0) {
    throw new Error(
        `A Hat settings differ from the original ASL: missing=${JSON.stringify(missingSettingKeys)}, `
        + `unexpected=${JSON.stringify(unexpectedSettingKeys)}`,
    );
}

for (let poll = 0; poll < 20; poll += 1) {
    const readsBeforePoll = reads.length;
    instance.exports.update();
    const pollReads = reads.slice(readsBeforePoll);
    readsPerPoll.push(pollReads);
    if (pollReads.filter(([, size]) => size > 8).length > 1) {
        throw new Error(
            `one update performed several scan windows: ${JSON.stringify(
                pollReads.map(([address, size]) => `${address.toString(16)}:${size}`),
            )}`,
        );
    }
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

console.log(JSON.stringify({
    pollsCompleted: readsPerPoll.length,
    reads: reads.length,
    registeredSettings: registeredBooleanKeys.size,
}));
