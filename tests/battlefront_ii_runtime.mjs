import fs from "node:fs";

const wasmPath = process.argv[2];
const mode = process.argv[3] ?? "other";
if (!wasmPath || !["other", "gc"].includes(mode)) {
    throw new Error("usage: node tests/battlefront_ii_runtime.mjs <battlefront-ii.wasm> [other|gc]");
}

const decoder = new TextDecoder();
const values = new Map();
if (mode === "gc") {
    values.set("gc", true);
    values.set("other", true);
}
const valueHandles = new Map();
const widgets = [];
const tooltips = new Map();
let nextValueHandle = 10n;
let instance;
let processOpen = true;
let victoryScreen = 0;
let loadingGame = 0;
let endGc = "playing";
let failEndGc = false;
let splits = 0;
let pauses = 0;
let resumes = 0;
let detaches = 0;
let stringReads = 0;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

function writeString(destination, size, value) {
    if (size !== 16) {
        throw new Error(`endGc read used ${size} bytes instead of 16`);
    }
    const output = new Uint8Array(instance.exports.memory.buffer, destination, size);
    output.fill("Z".charCodeAt(0));
    output.set(new TextEncoder().encode(value).subarray(0, size));
    if (value.length < output.length) {
        output[value.length] = 0;
        if (value.length + 2 < output.length) {
            // Data after the terminator must not participate in comparisons.
            output[value.length + 1] = "X".charCodeAt(0);
            output[value.length + 2] = "Y".charCodeAt(0);
        }
    }
    stringReads += 1;
}

const env = {
    timer_get_state: () => 1,
    timer_split() { splits += 1; },
    timer_pause_game_time() { pauses += 1; },
    timer_resume_game_time() { resumes += 1; },
    process_attach(pointer, length) {
        return text(pointer, length) === "BattlefrontII" ? 1n : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_read(_process, address, destination, size) {
        const view = new DataView(instance.exports.memory.buffer);
        if (address === 0x1aafca0n && size === 4) {
            view.setInt32(destination, victoryScreen, true);
        } else if (address === 0x05cefc0n && size === 4) {
            view.setInt32(destination, loadingGame, true);
        } else if (address === 0x1abda34n) {
            if (failEndGc) return 0;
            writeString(destination, size, endGc);
        } else {
            throw new Error(`unexpected process read ${address.toString(16)} (${size})`);
        }
        return 1;
    },
    user_settings_add_bool(keyPointer, keyLength, labelPointer, labelLength, defaultValue) {
        const key = text(keyPointer, keyLength);
        if (!values.has(key)) values.set(key, defaultValue !== 0);
        widgets.push([key, text(labelPointer, labelLength), defaultValue !== 0]);
        return values.get(key) ? 1 : 0;
    },
    user_settings_set_tooltip(keyPointer, keyLength, tooltipPointer, tooltipLength) {
        tooltips.set(text(keyPointer, keyLength), text(tooltipPointer, tooltipLength));
    },
    settings_map_load: () => 1n,
    settings_map_free() {},
    settings_map_get(_map, keyPointer, keyLength) {
        const key = text(keyPointer, keyLength);
        if (!values.has(key)) return 0n;
        const handle = nextValueHandle++;
        valueHandles.set(handle, values.get(key));
        return handle;
    },
    setting_value_free(handle) { valueHandles.delete(handle); },
    setting_value_get_bool(handle, outputPointer) {
        const value = valueHandles.get(handle);
        if (typeof value !== "boolean") return 0;
        new DataView(instance.exports.memory.buffer).setUint8(outputPointer, value ? 1 : 0);
        return 1;
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();

// Attach and initialize old/current, then observe the initial non-loading tick.
instance.exports.update();
instance.exports.update();

loadingGame = 1;
instance.exports.update();
loadingGame = 0;

if (mode === "other") {
    victoryScreen = 1;
    instance.exports.update();
    instance.exports.update();
    victoryScreen = 0;
    instance.exports.update();
    endGc = "ifs_freeform_end";
    instance.exports.update();
    if (splits !== 1) {
        throw new Error(`other-mode split behavior differed: ${splits}`);
    }
} else {
    endGc = "ifs_freeform_end";
    instance.exports.update();
    instance.exports.update();

    // A failed required-field refresh retains the last accepted string value.
    endGc = "playing";
    failEndGc = true;
    instance.exports.update();
    failEndGc = false;
    instance.exports.update();

    // Galactic Conquest takes precedence over the simultaneously enabled
    // Other setting, so victory does not count as loading in this mode.
    victoryScreen = 1;
    const pausesBeforeVictory = pauses;
    const resumesBeforeVictory = resumes;
    instance.exports.update();
    if (pauses !== pausesBeforeVictory || resumes !== resumesBeforeVictory + 1) {
        throw new Error(`Galactic Conquest setting did not take precedence: ${JSON.stringify({ pauses, resumes })}`);
    }
    if (splits !== 2) {
        throw new Error(`Galactic Conquest split behavior differed: ${splits}`);
    }
}

processOpen = false;
instance.exports.update();

const expectedWidgets = [
    ["gc", "Galactic Conquest", false],
    ["other", "Other Game Modes", true],
];
if (JSON.stringify(widgets) !== JSON.stringify(expectedWidgets)) {
    throw new Error(`settings registration differed: ${JSON.stringify(widgets)}`);
}
if (!tooltips.get("gc")?.startsWith("Galactic Conquest load-removal ruleset")) {
    throw new Error(`Galactic Conquest tooltip was not registered: ${JSON.stringify(Object.fromEntries(tooltips))}`);
}
if (!tooltips.get("other")?.startsWith("Other game modes load-removal ruleset")) {
    throw new Error(`Other tooltip was not registered: ${JSON.stringify(Object.fromEntries(tooltips))}`);
}
if (pauses === 0 || resumes === 0 || detaches !== 1 || stringReads === 0 || valueHandles.size !== 0) {
    throw new Error(`runtime invariants differed: ${JSON.stringify({ pauses, resumes, detaches, stringReads, leakedHandles: valueHandles.size })}`);
}

console.log(JSON.stringify({ mode, splits, pauses, resumes, detaches, stringReads }));
