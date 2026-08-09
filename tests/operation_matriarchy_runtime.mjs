import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/operation_matriarchy_runtime.mjs <autosplitter.wasm>");
}

const decoder = new TextDecoder();
const encoder = new TextEncoder();
const gameModule = 0x10000000n;
const levelPictureAddress = gameModule + 0x992b4n;
let instance;
let processOpen = true;
let timerState = 0;
let levelPicture = "01_01_0.dds";
let reads = 0;
let starts = 0;
let splits = 0;
let resets = 0;
let pauses = 0;
let resumes = 0;
let detaches = 0;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

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
        return text(pointer, length) === "GAME.exe" ? 1n : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_get_module_address(_process, pointer, length) {
        if (text(pointer, length) !== "Game.dll") {
            throw new Error("queried an unexpected module");
        }
        return gameModule;
    },
    process_read(_process, address, destination, size) {
        reads += 1;
        if (address !== levelPictureAddress || size !== 12) {
            throw new Error(`unexpected process read ${address.toString(16)} (${size})`);
        }
        writeString(destination, size, levelPicture);
        return 1;
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();

// Seed the first snapshot without producing a lifecycle action.
instance.exports.update();
if (starts !== 0 || splits !== 0 || resets !== 0) {
    throw new Error("state initialization emitted a lifecycle action");
}

levelPicture = "01_01_1.dds";
instance.exports.update();
levelPicture = "01_02_1.dds";
instance.exports.update();
levelPicture = "01_01_0.dds";
instance.exports.update();

if (starts !== 1 || splits !== 1 || resets !== 1) {
    throw new Error(
        `autosplitter behavior differed: ${JSON.stringify({ starts, splits, resets })}`,
    );
}
if (pauses === 0 || resumes === 0) {
    throw new Error(`loading state was not reported: ${JSON.stringify({ pauses, resumes })}`);
}
if (reads !== 4) {
    throw new Error(`unexpected level-picture read count: ${reads}`);
}

processOpen = false;
instance.exports.update();
if (detaches !== 1) {
    throw new Error(`expected one detach, got ${detaches}`);
}

console.log(JSON.stringify({ reads, starts, splits, resets, pauses, resumes, detaches }));
