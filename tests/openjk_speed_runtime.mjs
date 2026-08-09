import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/openjk_speed_runtime.mjs <openjk-speed.wasm>");
}

const decoder = new TextDecoder();
const encoder = new TextEncoder();
const activeAddress = 0x407aa4n;
const levelAddress = 0x40e10cn;
let instance;
let processOpen = true;
let isActive = true;
let level = "maps/yavin1b.bsp";
let timerState = 0;
let starts = 0;
let splits = 0;
let resets = 0;
let pauses = 0;
let resumes = 0;
let attaches = 0;
let detaches = 0;
let stringReads = 0;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

function writeLevel(destination, size) {
    if (size !== 30) {
        throw new Error(`level read used ${size} bytes instead of 30`);
    }
    const encoded = encoder.encode(level);
    if (encoded.length >= size) {
        throw new Error(`fixture level exceeds its ${size}-byte state field`);
    }
    const output = new Uint8Array(instance.exports.memory.buffer, destination, size);
    output.fill(0);
    output.set(encoded);
    stringReads += 1;
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
        if (text(pointer, length) !== "openjk_sp.x86") return 0n;
        attaches += 1;
        return BigInt(attaches);
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_read(_process, address, destination, size) {
        const view = new DataView(instance.exports.memory.buffer);
        if (address === activeAddress && size === 1) {
            view.setUint8(destination, isActive ? 1 : 0);
        } else if (address === levelAddress) {
            writeLevel(destination, size);
        } else {
            throw new Error(`unexpected process read ${address.toString(16)} (${size})`);
        }
        return 1;
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();

// Attach and seed old/current, then start on the stable opening map.
instance.exports.update();
instance.exports.update();
if (starts !== 1 || splits !== 0) {
    throw new Error(`opening-map start differed: ${JSON.stringify({ starts, splits })}`);
}

level = "maps/level1.bsp";
instance.exports.update();
level = "maps/academy1.bsp";
instance.exports.update();
level = "maps/level2.bsp";
instance.exports.update();
level = "maps/level1.bsp";
instance.exports.update();
level = "";
instance.exports.update();
level = "maps/level3.bsp";
instance.exports.update();
if (splits !== 3) {
    throw new Error(`visited/ignored-map filtering differed: ${splits} splits`);
}

const pausesBeforeLoading = pauses;
isActive = false;
instance.exports.update();
if (pauses !== pausesBeforeLoading + 1) {
    throw new Error(`loading did not pause game time: ${JSON.stringify({ pauses, resumes })}`);
}
const resumesBeforeLoading = resumes;
isActive = true;
instance.exports.update();
if (resumes !== resumesBeforeLoading + 1) {
    throw new Error(`leaving loading did not resume game time: ${JSON.stringify({ pauses, resumes })}`);
}

// Returning to the inactive opening map resets. Once the next run starts, its
// first stable opening-map tick evaluates the original reset-block cleanup.
isActive = false;
level = "maps/yavin1b.bsp";
instance.exports.update();
instance.exports.update();
isActive = true;
instance.exports.update();
instance.exports.update();
level = "maps/level1.bsp";
instance.exports.update();
if (resets !== 1 || starts !== 2 || splits !== 4) {
    throw new Error(`new-run reset behavior differed: ${JSON.stringify({ starts, splits, resets })}`);
}

// The ASL init boundary is per attachment. Reattaching therefore clears maps
// even while LiveSplit's timer remains running.
level = "maps/level2.bsp";
instance.exports.update();
processOpen = false;
instance.exports.update();
processOpen = true;
instance.exports.update();
level = "maps/level1.bsp";
instance.exports.update();

if (splits !== 6 || attaches !== 2 || detaches !== 1 || stringReads === 0) {
    throw new Error(`reattach behavior differed: ${JSON.stringify({ splits, attaches, detaches, stringReads })}`);
}

console.log(JSON.stringify({ starts, splits, resets, pauses, resumes, attaches, detaches, stringReads }));
