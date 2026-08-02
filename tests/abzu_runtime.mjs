import fs from "node:fs";

const wasmPath = process.argv[2];
const mode = process.argv[3] ?? "steam";
const epic = mode === "epic";
const unknown = mode === "unknown";
if (!wasmPath) {
    throw new Error("usage: node tests/abzu_runtime.mjs <abzu.wasm> [epic|unknown]");
}

const bytes = fs.readFileSync(wasmPath);
const decoder = new TextDecoder();
const moduleSize = unknown ? 1n : epic ? 47_501_312n : 47_570_944n;
const loadingOffset = epic ? 0x028ea104n : 0x029020f4n;
const expectedBuild = epic ? "Epic" : "Steam";
const moduleBase = 0x10000000n;
const moduleNames = [];
const messages = [];
const variables = new Map();
let instance;
let loading = 0;
let splits = 0;
let processOpen = true;
let detaches = 0;

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
    process_attach(pointer, length) {
        return text(pointer, length) === "AbzuGame-Win64-Shipping.exe" ? 1n : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_read(_process, address, destination, size) {
        if (address !== moduleBase + loadingOffset || size !== 4) {
            throw new Error(`unexpected loading read: ${address.toString(16)} (${size})`);
        }
        new DataView(instance.exports.memory.buffer).setInt32(destination, loading, true);
        return 1;
    },
    process_get_module_address(_process, pointer, length) {
        moduleNames.push(text(pointer, length));
        return moduleBase;
    },
    process_get_module_size(_process, pointer, length) {
        moduleNames.push(text(pointer, length));
        return moduleSize;
    },
    runtime_print_message(pointer, length) {
        messages.push(text(pointer, length));
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

({ instance } = await WebAssembly.instantiate(bytes, { env }));
instance.exports._start();

instance.exports.update();
loading = 1;
instance.exports.update();
if (unknown) {
    processOpen = false;
    instance.exports.update();
}

if (moduleNames.join(",") !== [
    "AbzuGame-Win64-Shipping.exe",
    "AbzuGame-Win64-Shipping.exe",
].join(",")) {
    throw new Error(`unexpected main-module queries: ${JSON.stringify(moduleNames)}`);
}
if (!unknown && variables.get("Build") !== expectedBuild) {
    throw new Error(`unexpected selected build: ${JSON.stringify(Object.fromEntries(variables))}`);
}
if (unknown && (variables.has("Build") || splits !== 0 || detaches !== 1
    || messages.join(",") !== "unsupported ABZÛ module size 1")) {
    throw new Error(`unsupported build lifetime was incorrect: ${JSON.stringify({ messages, splits, detaches, variables: Object.fromEntries(variables) })}`);
}
if (!unknown && splits !== 1) {
    throw new Error(`expected one loading transition split, got ${splits}`);
}

console.log(JSON.stringify({ build: unknown ? "unsupported" : expectedBuild, moduleNames, splits }));
