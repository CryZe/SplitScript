import fs from "node:fs";

const wasmPath = process.argv[2];
const mode = process.argv[3] ?? "full";
if (!wasmPath || !["full", "skipped"].includes(mode)) {
    throw new Error("usage: node tests/dark_sasi_runtime.mjs <dark-sasi.wasm> [full|skipped]");
}

const readings = [
    1_000_000_000n,
    2_000_000_000n,
    3_000_000_000n,
    10_000_000_000n,
    11_000_000_000n,
    62_999_999_999n,
    63_000_000_000n,
];
let instance;
let processOpen = true;
let timerState = 0;
let currentSplitIndex = -1n;
let level = 0;
let starts = 0;
let splits = 0;
let pauses = 0;
let resumes = 0;
let detaches = 0;
let clockCalls = 0;

const env = {
    timer_get_state: () => timerState,
    timer_current_split_index: () => currentSplitIndex,
    timer_start() {
        starts += 1;
        timerState = 1;
        currentSplitIndex = 0n;
    },
    timer_split() {
        splits += 1;
        currentSplitIndex += 1n;
    },
    timer_pause_game_time() { pauses += 1; },
    timer_resume_game_time() { resumes += 1; },
    process_attach: () => 1n,
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_read(_process, address, destination, size) {
        if (address !== 0x293c1d8n || size !== 4) {
            throw new Error(`unexpected process read ${address.toString(16)} (${size})`);
        }
        new DataView(instance.exports.memory.buffer).setInt32(destination, level, true);
        return 1;
    },
};

const wasi = {
    clock_time_get(clockId, precision, destination) {
        if (clockId !== 1 || precision !== 1n || clockCalls >= readings.length) {
            return 28;
        }
        new DataView(instance.exports.memory.buffer).setBigUint64(
            destination,
            readings[clockCalls],
            true,
        );
        clockCalls += 1;
        return 0;
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), {
    env,
    wasi_snapshot_preview1: wasi,
}));
instance.exports._start();

// Initialize at level zero, then trigger the original 0 -> 15 start edge.
instance.exports.update();
level = 15;
instance.exports.update();
if (starts !== 1 || currentSplitIndex !== 0n) {
    throw new Error(`start behavior differed: ${JSON.stringify({ starts, currentSplitIndex: String(currentSplitIndex) })}`);
}

// Even with an active timer, a negative host index is exposed as None. Loading
// still follows the level-zero state and does not accidentally advance a split.
currentSplitIndex = -1n;
level = 0;
instance.exports.update();
level = 15;
instance.exports.update();
if (splits !== 0 || pauses === 0 || resumes === 0) {
    throw new Error(`None/loading behavior differed: ${JSON.stringify({ splits, pauses, resumes })}`);
}

if (mode === "full") {
    currentSplitIndex = 0n;
    level = 8;
    instance.exports.update();
} else {
    // A skipped first segment advances the host index without invoking split.
    currentSplitIndex = 1n;
    level = 8;
    instance.exports.update();
}

level = 12;
instance.exports.update();
level = 5;
instance.exports.update();
instance.exports.update();
level = 8;
instance.exports.update();

const expectedFirstRunSplits = mode === "full" ? 3 : 2;
if (splits !== expectedFirstRunSplits || currentSplitIndex !== 3n || clockCalls !== 3) {
    throw new Error(`split-index dispatch differed: ${JSON.stringify({ mode, splits, currentSplitIndex: String(currentSplitIndex), clockCalls })}`);
}

// Simulate a manual reset before the delayed final split. The next start must
// discard the pending timestamp, just like Stopwatch.Reset in the source ASL.
timerState = 0;
currentSplitIndex = -1n;
level = 0;
instance.exports.update();
level = 15;
instance.exports.update();
currentSplitIndex = 3n;
level = 9;
instance.exports.update();
if (starts !== 2 || splits !== expectedFirstRunSplits || clockCalls !== 3) {
    throw new Error(`restart did not clear the pending delay: ${JSON.stringify({ starts, splits, clockCalls })}`);
}

// Re-enter index 2. Repeated polls restart the timestamp; the index-2 split
// retains 11 seconds, so 62.999999999 seconds is early and 63 seconds is exact.
currentSplitIndex = 2n;
level = 5;
instance.exports.update();
level = 8;
instance.exports.update();
level = 9;
instance.exports.update();
const splitsBeforeDeadline = splits;
instance.exports.update();
if (splits !== splitsBeforeDeadline + 1 || clockCalls !== readings.length) {
    throw new Error(`52-second final delay differed: ${JSON.stringify({ splits, splitsBeforeDeadline, clockCalls })}`);
}

instance.exports.update();
if (splits !== splitsBeforeDeadline + 1 || clockCalls !== readings.length) {
    throw new Error("completed final delay was not cleared");
}

processOpen = false;
instance.exports.update();
if (detaches !== 1) {
    throw new Error(`detach behavior differed: ${detaches}`);
}

console.log(JSON.stringify({ mode, starts, splits, pauses, resumes, detaches, clockCalls }));
