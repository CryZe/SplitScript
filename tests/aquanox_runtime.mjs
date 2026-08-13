import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/aquanox_runtime.mjs <autosplitter.wasm>");
}

const decoder = new TextDecoder();
const encoder = new TextEncoder();
const mainModule = 0x10000000n;
const binkModule = 0x11000000n;
const pointers = new Map([
    [mainModule + 0x10dbc0n, 0x20000000n],
    [mainModule + 0x292f5cn, 0x21000000n],
    [0x21000008n, 0x21010000n],
    [mainModule + 0x263258n, 0x22000000n],
    [0x2200001cn, 0x22010000n],
    [mainModule + 0x2676c0n, 0x23000000n],
    [0x2300007cn, 0x23010000n],
    [0x23010040n, 0x23020000n],
    [mainModule + 0x26323cn, 0x24000000n],
    [0x240000a0n, 0x24010000n],
    [0x240102f4n, 0x24020000n],
    [0x24010340n, 0x24030000n],
]);

let instance;
let processOpen = true;
let timerState = 0;
let cutscene = 1;
let loading = 43;
let activeStation = 0;
let nextStation = 2;
let mapPath = "map\\ordinary";
let menu1 = "Other";
let menu2 = "Other";
let menu2Missing = false;
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
        throw new Error(`fixture string exceeds its ${size}-byte state field`);
    }
    const output = new Uint8Array(instance.exports.memory.buffer, destination, size);
    output.fill(0);
    output.set(encoded);
}

const env = {
    timer_get_state: () => timerState,
    runtime_set_tick_rate() {},
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
        return text(pointer, length) === "Aqua" ? 1n : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_get_module_address(_process, pointer, length) {
        const name = text(pointer, length);
        if (name === "Aqua") return mainModule;
        if (name === "binkw32.dll") return binkModule;
        return 0n;
    },
    process_read(_process, address, destination, size) {
        if (pointers.has(address)) {
            if (size !== 8) throw new Error(`pointer read used ${size} bytes`);
            writePointer(destination, pointers.get(address));
            return 1;
        }

        const view = new DataView(instance.exports.memory.buffer);
        if (address === binkModule + 0x5e064n && size === 4) {
            view.setInt32(destination, cutscene, true);
        } else if (address === 0x200001d8n && size === 1) {
            view.setUint8(destination, loading);
        } else if (address === 0x21010028n && size === 1) {
            view.setUint8(destination, activeStation);
        } else if (address === 0x22010028n && size === 1) {
            view.setUint8(destination, nextStation);
        } else if (address === 0x23020060n && size === 64) {
            writeString(destination, size, mapPath);
        } else if (address === 0x24020000n && size === 32) {
            writeString(destination, size, menu1);
        } else if (address === 0x24030000n && size === 32) {
            if (menu2Missing) return 0;
            writeString(destination, size, menu2);
        } else {
            throw new Error(`unexpected process read ${address.toString(16)} (${size})`);
        }
        return 1;
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();

// Seed old/current without triggering start, split, reset, or load actions.
instance.exports.update();

cutscene = 0;
loading = 0;
menu1 = "Dipol Auto Activate: 0";
menu2 = "Activate Dipol";
instance.exports.update();

menu1 = "Other";
instance.exports.update();

menu2Missing = true;
instance.exports.update();

menu2Missing = false;
menu1 = "Dipol Auto Activate: 20";
mapPath = "map\\6h4\\script\\6h4";
instance.exports.update();

cutscene = 1;
menu1 = "Other";
instance.exports.update();

if (starts !== 1 || splits !== 3 || resets !== 1) {
    throw new Error(`Aquanox behavior differed: ${JSON.stringify({ starts, splits, resets })}`);
}

const pausesBeforeDetach = pauses;
processOpen = false;
instance.exports.update();
if (detaches !== 1 || pauses !== pausesBeforeDetach + 1) {
    throw new Error(`detach cleanup differed: ${JSON.stringify({ detaches, pauses })}`);
}

console.log(JSON.stringify({ starts, splits, resets, pauses, resumes, detaches }));
