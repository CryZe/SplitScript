import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/wrapper_fallbacks_runtime.mjs <fallbacks.wasm>");
}

const bytes = fs.readFileSync(wasmPath);
const decoder = new TextDecoder();
let instance;
let observed;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 0,
    timer_start() {},
    timer_split() {},
    timer_reset() {},
    timer_set_game_time() {},
    timer_pause_game_time() {},
    timer_resume_game_time() {},
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        if (text(keyPointer, keyLength) === "Fallbacks") {
            observed = text(valuePointer, valueLength);
        }
    },
    runtime_set_tick_rate() {},
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    process_read: () => 1,
    process_get_module_address: () => 0n,
    process_get_module_size: () => 0n,
    runtime_print_message() {},
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
for (let tick = 0; tick < 3 && observed === undefined; tick += 1) {
    instance.exports.update();
}

const expected = "41,3,13,17,6,23,7,31";
if (observed !== expected) {
    throw new Error(`unexpected fallback values: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify({ observed }));
