import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/tiberian_sun_runtime.mjs <tiberian-sun.wasm>");
}

const moduleBase = 0x10000000n;
const splashPointer = 0x20000000n;
const decoder = new TextDecoder();
const encoder = new TextEncoder();
const processNames = [];
const moduleNames = [];
let instance;
let processOpen = true;
let timerState = 0;
let isPlaying = 1;
let mainMenuIndex = 1;
let gameState = 0;
let splashVisible = 0;
let splashText = "";
let starts = 0;
let splits = 0;
let pauses = 0;
let resumes = 0;
let detaches = 0;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

function writeSplash(destination) {
    const output = new Uint8Array(instance.exports.memory.buffer, destination, 20);
    output.fill(0);
    const encoded = encoder.encode(splashText);
    if (encoded.length > output.length) {
        throw new Error(`fixture splash text exceeds string20: ${splashText}`);
    }
    output.set(encoded);
}

const env = {
    timer_get_state: () => timerState,
    timer_start() { starts += 1; timerState = 1; },
    timer_split() { splits += 1; },
    timer_reset() {},
    timer_set_game_time() {},
    timer_pause_game_time() { pauses += 1; },
    timer_resume_game_time() { resumes += 1; },
    timer_set_variable() {},
    runtime_set_tick_rate() {},
    runtime_print_message() {},
    process_attach(pointer, length) {
        const name = text(pointer, length);
        processNames.push(name);
        return name === "game.exe" ? 1n : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_get_module_address(_process, pointer, length) {
        const name = text(pointer, length);
        moduleNames.push(name);
        return name === "game.exe" ? moduleBase : 0n;
    },
    process_read(_process, address, destination, size) {
        const view = new DataView(instance.exports.memory.buffer);
        if (address === moduleBase + 0x3e48fcn && size === 1) {
            view.setUint8(destination, isPlaying);
        } else if (address === moduleBase + 0x408c4cn && size === 1) {
            view.setUint8(destination, mainMenuIndex);
        } else if (address === moduleBase + 0x3e224cn && size === 1) {
            view.setUint8(destination, gameState);
        } else if (address === moduleBase + 0x34c5f4n && size === 8) {
            view.setBigUint64(destination, splashPointer, true);
        } else if (address === splashPointer + 0x14n && size === 1) {
            view.setUint8(destination, splashVisible);
        } else if (address === splashPointer + 0x14n && size === 20) {
            writeSplash(destination);
        } else {
            throw new Error(`unexpected Tiberian Sun read ${address.toString(16)} (${size})`);
        }
        return 1;
    },
    user_settings_add_bool: () => 1,
    user_settings_add_title() {},
    user_settings_add_choice() {},
    user_settings_add_choice_option: () => 0,
    user_settings_add_file_select() {},
    user_settings_add_file_select_name_filter() {},
    user_settings_add_file_select_mime_filter() {},
    user_settings_set_tooltip() {},
    settings_map_load: () => 1n,
    settings_map_free() {},
    settings_map_get: () => 0n,
    setting_value_free() {},
    setting_value_get_bool: () => 0,
    setting_value_get_string: () => 0,
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
instance.exports.update();

mainMenuIndex = 0;
gameState = 255;
instance.exports.update();
if (starts !== 1) throw new Error(`start transition differed: ${starts}`);

isPlaying = 0;
instance.exports.update();
isPlaying = 1;
instance.exports.update();

splashText = "MISSION ACCOMPLISHED";
splashVisible = 1;
instance.exports.update();
splashVisible = 0;
instance.exports.update();
splashText = "MISION CUMPLIDA";
splashVisible = 1;
instance.exports.update();
splashVisible = 0;
instance.exports.update();
splashText = "MISSION FAILED";
splashVisible = 1;
instance.exports.update();

if (splits !== 2 || pauses !== 1 || resumes !== 6) {
    throw new Error(`timer behavior differed: ${JSON.stringify({ splits, pauses, resumes })}`);
}
if (processNames.join(",") !== "game.exe" || moduleNames.some(name => name !== "game.exe")) {
    throw new Error(`process/module identity differed: ${JSON.stringify({ processNames, moduleNames })}`);
}

processOpen = false;
instance.exports.update();
if (detaches !== 1) throw new Error(`detach behavior differed: ${detaches}`);

console.log(JSON.stringify({ starts, splits, pauses, resumes, detaches, moduleLookups: moduleNames.length }));
