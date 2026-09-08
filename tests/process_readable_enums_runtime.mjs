import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/process_readable_enums_runtime.mjs <fixture.wasm>");
}

const bytes = fs.readFileSync(wasmPath);
const decoder = new TextDecoder();
let instance;
const variables = new Map();

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
        variables.set(text(keyPointer, keyLength), text(valuePointer, valueLength));
    },
    runtime_set_tick_rate() {},
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    process_read(_process, address, destination, size) {
        const view = new DataView(instance.exports.memory.buffer);
        switch (Number(address)) {
            case 0x100:
                if (size !== 4) throw new Error(`unexpected enum size: ${size}`);
                view.setInt32(destination, 6, true);
                return 1;
            case 0x200:
                if (size !== 12) throw new Error(`unexpected nested size: ${size}`);
                view.setInt32(destination, 2, true);
                view.setInt32(destination + 4, 0, true);
                view.setInt32(destination + 8, 7, true);
                return 1;
            case 0x300:
                view.setInt32(destination, 99, true);
                return 1;
            case 0x400:
                if (size !== 12) throw new Error(`unexpected invalid nested size: ${size}`);
                view.setInt32(destination, 2, true);
                view.setInt32(destination + 4, 99, true);
                view.setInt32(destination + 8, 7, true);
                return 1;
            default:
                throw new Error(`unexpected process read at 0x${Number(address).toString(16)}`);
        }
    },
    runtime_print_message() {},
    user_settings_add_bool: () => 1,
    user_settings_add_title() {},
    user_settings_add_choice() {},
    user_settings_add_choice_option: () => 0,
    user_settings_add_file_select() {},
    user_settings_add_file_select_name_filter() {},
    user_settings_add_file_select_mime_filter() {},
    user_settings_add_text_input: () => 0,
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
instance.exports.update();

const observed = Object.fromEntries(variables);
const expected = {
    Direct: "GameState.Results",
    Nested: "Snapshot {\n    state: GameState.Menu,\n    history: [\n        GameState.Mission,\n        GameState.Load,\n    ],\n}",
    Invalid: "GameState.Mission",
    "Invalid Nested": "Snapshot {\n    state: GameState.TitleScreen,\n    history: [\n        GameState.TitleScreen,\n        GameState.TitleScreen,\n    ],\n}",
};
if (JSON.stringify(observed) !== JSON.stringify(expected)) {
    throw new Error(`unexpected readable enums: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify({ observed }));
