import fs from "node:fs";

const wasmPath = process.argv[2];
const mode = process.argv[3] ?? "steam";
if (!wasmPath) {
    throw new Error("usage: node tests/state_layouts_runtime.mjs <state-layouts.wasm> [gog|unknown]");
}

const unknown = mode === "unknown";
const gog = mode === "gog";
const moduleSize = unknown ? 1n : gog ? 0x2000n : 0x1000n;
const expectedAddress = gog ? 0x2000n : 0x1000n;
const expectedLayout = gog ? "GOG" : "Steam";
const decoder = new TextDecoder();
const variables = new Map();
const reads = [];
let instance;
let memoryValue = 0;
let processOpen = true;
let detaches = 0;
let splits = 0;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 1,
    timer_start() {},
    timer_split() { splits += 1; },
    timer_reset() {},
    timer_set_game_time() {},
    timer_pause_game_time() {},
    timer_resume_game_time() {},
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        variables.set(text(keyPointer, keyLength), text(valuePointer, valueLength));
    },
    runtime_set_tick_rate() {},
    runtime_print_message() {},
    process_attach(pointer, length) {
        return text(pointer, length) === "LayoutGame.exe" ? 1n : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_read(_process, address, destination, size) {
        reads.push(address);
        if (unknown || address !== expectedAddress || size !== 4) {
            throw new Error(`unexpected state read: ${address.toString(16)} (${size})`);
        }
        new DataView(instance.exports.memory.buffer).setUint32(destination, memoryValue, true);
        return 1;
    },
    process_get_module_address: () => 0x400000n,
    process_get_module_size: () => moduleSize,
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

if (unknown) {
    processOpen = false;
    instance.exports.update();
    if (reads.length !== 0 || variables.size !== 0 || splits !== 0 || detaches !== 1) {
        throw new Error(`unsupported layout was not inert: ${JSON.stringify({ reads, variables: Object.fromEntries(variables), splits, detaches })}`);
    }
} else {
    memoryValue = 1;
    instance.exports.update();
    if (reads.some((address) => address !== expectedAddress)) {
        throw new Error(`selected layout read the wrong address: ${reads.map(String)}`);
    }
    if (variables.get("Layout") !== expectedLayout || splits !== 1) {
        throw new Error(`selected layout behaved incorrectly: ${JSON.stringify({ variables: Object.fromEntries(variables), splits })}`);
    }
}

console.log(JSON.stringify({ mode, reads: reads.map((address) => Number(address)), splits, detaches }));
