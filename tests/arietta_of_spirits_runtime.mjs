import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/arietta_of_spirits_runtime.mjs <autosplitter.wasm>");
}

const moduleBase = 0x10000000n;
const stagePointer = 0x20000000n;
const pausePointer1 = 0x30000000n;
const pausePointer2 = 0x31000000n;
const pausePointer3 = 0x32000000n;
const stageRoot = moduleBase + 0x0261462cn;
const stageAddress = stagePointer + 0x100n;
const pauseRoot = moduleBase + 0x023d6db4n;
const pauseAddress = pausePointer3;
const decoder = new TextDecoder();
const encoder = new TextEncoder();
const reads = [];
let instance;
let processOpen = true;
let timerState = 0;
let speedrunStage = "SPEEDRUN_NONE";
let pauseMenuOpen = "NO";
let failStageRead = false;
let starts = 0;
let splits = 0;
let resets = 0;
let pauses = 0;
let resumes = 0;
let detaches = 0;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

function writePointer(destination, value) {
    new DataView(instance.exports.memory.buffer).setBigUint64(destination, value, true);
}

function writeString(destination, size, value) {
    const encoded = encoder.encode(value);
    if (encoded.length >= size) {
        throw new Error(`fixture string exceeds its ${size}-byte field`);
    }
    const output = new Uint8Array(instance.exports.memory.buffer, destination, size);
    output.fill(0);
    output.set(encoded);
}

const env = {
    timer_get_state: () => timerState,
    timer_start() {
        starts += 1;
        timerState = 1;
    },
    timer_split() { splits += 1; },
    timer_reset() {
        resets += 1;
        timerState = 0;
    },
    timer_pause_game_time() { pauses += 1; },
    timer_resume_game_time() { resumes += 1; },
    process_attach(pointer, length) {
        return text(pointer, length) === "Arietta of Spirits" ? 1n : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_get_module_address(_process, pointer, length) {
        if (text(pointer, length) !== "Arietta of Spirits") {
            throw new Error("queried an unexpected module");
        }
        return moduleBase;
    },
    process_read(_process, address, destination, size) {
        reads.push([address, size]);
        if (address === stageRoot && size === 8) {
            writePointer(destination, stagePointer);
        } else if (address === stageAddress && size === 128) {
            if (failStageRead) return 0;
            writeString(destination, size, speedrunStage);
        } else if (address === pauseRoot && size === 8) {
            writePointer(destination, pausePointer1);
        } else if (address === pausePointer1 + 0x14n && size === 8) {
            writePointer(destination, pausePointer2);
        } else if (address === pausePointer2 + 0xcn && size === 8) {
            writePointer(destination, pausePointer3);
        } else if (address === pauseAddress && size === 8) {
            writeString(destination, size, pauseMenuOpen);
        } else {
            throw new Error(`unexpected process read ${address.toString(16)} (${size})`);
        }
        return 1;
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();

// The first complete poll seeds old and current without lifecycle actions.
instance.exports.update();
if (starts !== 0 || splits !== 0 || resets !== 0) {
    throw new Error("state initialization emitted a lifecycle action");
}

speedrunStage = "SPEEDRUN_STAGE_01";
instance.exports.update();
speedrunStage = "SPEEDRUN_STAGE_02";
instance.exports.update();

// A failed stage-string read retains that field while the independently read
// pause flag still advances and removes loading time.
speedrunStage = "SPEEDRUN_STAGE_03";
pauseMenuOpen = "YES";
failStageRead = true;
instance.exports.update();
if (splits !== 1 || pauses !== 1) {
    throw new Error(`failed field did not retain independently: ${JSON.stringify({ splits, pauses })}`);
}
failStageRead = false;
instance.exports.update();

pauseMenuOpen = "NO";
instance.exports.update();
speedrunStage = "SPEEDRUN_NONE";
instance.exports.update();
speedrunStage = "SPEEDRUN_STAGE_04";
instance.exports.update();

if (starts !== 2 || splits !== 2 || resets !== 1 || pauses !== 2 || resumes !== 3) {
    throw new Error(
        `autosplitter behavior differed: ${JSON.stringify({ starts, splits, resets, pauses, resumes })}`,
    );
}
if (reads.length !== 48) {
    throw new Error(`unexpected pointer/string read count: ${reads.length}`);
}

processOpen = false;
instance.exports.update();
if (detaches !== 1) {
    throw new Error(`expected one detach, got ${detaches}`);
}

console.log(JSON.stringify({ reads: reads.length, starts, splits, resets, pauses, resumes, detaches }));
