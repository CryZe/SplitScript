import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/module_path_runtime.mjs <module-path.wasm>");
}

const decoder = new TextDecoder();
const encoder = new TextEncoder();
const variables = new Map();
const pathQueries = [];
const messages = [];
const moduleBase = 0x10000000n;
let instance;

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
    runtime_print_message(pointer, length) { messages.push(text(pointer, length)); },
    process_attach(pointer, length) {
        return text(pointer, length) === "game-demo.exe" ? 7n : 0n;
    },
    process_detach() {},
    process_is_open: () => 1,
    process_get_module_address: () => moduleBase,
    process_get_module_size: () => 0x1000n,
    process_read(_process, address, destination, size) {
        const view = new DataView(instance.exports.memory.buffer);
        const bytes = new Uint8Array(instance.exports.memory.buffer, destination, size);
        bytes.fill(0);
        if (address === moduleBase && size === 2) {
            view.setUint16(destination, 0x5a4d, true);
        } else if (address === moduleBase + 0x3cn && size === 4) {
            view.setUint32(destination, 0x80, true);
        } else if (address === moduleBase + 0x80n && size === 4) {
            view.setUint32(destination, 0x00004550, true);
        } else if (address === moduleBase + 0x98n && size === 2) {
            view.setUint16(destination, 0x020b, true);
        } else if (address === moduleBase + 0x118n && size === 8) {
            view.setUint32(destination, 0x200, true);
            view.setUint32(destination + 4, 0x400, true);
        } else if (address === moduleBase + 0x200n && size === 16) {
            view.setUint16(destination + 14, 2, true);
        } else if (address === moduleBase + 0x210n && size === 4) {
            view.setUint32(destination, 3, true);
        } else if (address === moduleBase + 0x214n && size === 4) {
            view.setUint32(destination, 0x80000030, true);
        } else if (address === moduleBase + 0x218n && size === 4) {
            view.setUint32(destination, 0x10, true);
        } else if (address === moduleBase + 0x21cn && size === 4) {
            view.setUint32(destination, 0x80000040, true);
        } else if (address === moduleBase + 0x240n && size === 16) {
            view.setUint16(destination + 14, 1, true);
        } else if (address === moduleBase + 0x254n && size === 4) {
            view.setUint32(destination, 0x80000070, true);
        } else if (address === moduleBase + 0x270n && size === 16) {
            view.setUint16(destination + 14, 1, true);
        } else if (address === moduleBase + 0x284n && size === 4) {
            view.setUint32(destination, 0xa0, true);
        } else if (address === moduleBase + 0x2a0n && size === 4) {
            view.setUint32(destination, 0x300, true);
        } else if (address === moduleBase + 0x328n && size === 4) {
            view.setUint32(destination, 0xfeef04bd, true);
        } else if (address === moduleBase + 0x330n && size === 16) {
            view.setUint16(destination, 2, true);
            view.setUint16(destination + 2, 1, true);
            view.setUint16(destination + 4, 4, true);
            view.setUint16(destination + 6, 3, true);
            view.setUint16(destination + 8, 6, true);
            view.setUint16(destination + 10, 5, true);
            view.setUint16(destination + 12, 8, true);
            view.setUint16(destination + 14, 7, true);
        } else {
            throw new Error(`unexpected PE read ${address.toString(16)} (${size})`);
        }
        return 1;
    },
    process_get_module_path(_process, namePointer, nameLength, pathPointer, lengthPointer) {
        const name = text(namePointer, nameLength);
        pathQueries.push(name);
        const path = name === "game-demo.exe" ? "/games/demo/game-demo.exe" : null;
        const view = new DataView(instance.exports.memory.buffer);
        if (path === null) {
            view.setUint32(lengthPointer, 0, true);
            return 0;
        }
        const bytes = encoder.encode(path);
        view.setUint32(lengthPointer, bytes.length, true);
        if (pathPointer === 0) {
            return 0;
        }
        new Uint8Array(instance.exports.memory.buffer, pathPointer, bytes.length).set(bytes);
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

if (variables.get("Executable Path") !== "/games/demo/game-demo.exe"
    || variables.get("Plugin Path") !== "Unavailable"
    || variables.get("File Version") !== "1.2.3.4"
    || variables.get("Product Version") !== "5.6.7.8"
    || variables.get("Version Pair") !== "1.2.3.4 / 5.6.7.8"
    || variables.get("Casted File Version") !== "1.2.3.4"
    || messages.join(",") !== "1.2.3.4") {
    throw new Error(`unexpected module paths: ${JSON.stringify(Object.fromEntries(variables))}`);
}
if (pathQueries.join(",") !== "game-demo.exe,game-demo.exe,plugin.dll") {
    throw new Error(`module identity was not retained: ${JSON.stringify(pathQueries)}`);
}

console.log(JSON.stringify({ variables: Object.fromEntries(variables), pathQueries, messages }));
