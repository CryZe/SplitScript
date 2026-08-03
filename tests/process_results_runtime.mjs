import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/process_results_runtime.mjs <results.wasm>");
}

const bytes = fs.readFileSync(wasmPath);
const decoder = new TextDecoder();
let instance;
let phase = 0;
const snapshots = [];

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
        if (text(keyPointer, keyLength) === "Snapshot") {
            snapshots.push(text(valuePointer, valueLength));
        }
    },
    runtime_set_tick_rate() {},
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    process_read(_process, address, destination, size) {
        const numericAddress = Number(address);
        if (numericAddress === 0x3000
            || (phase === 2 && numericAddress === 0x7000)
            || (phase === 1 && (numericAddress === 0x2000 || numericAddress === 0x5000))) {
            return 0;
        }

        const value = numericAddress === 0x1000
            ? 10 + phase
            : numericAddress === 0x2000
                ? 20 + phase
                : numericAddress === 0x4000
                    ? 40 + phase
                    : numericAddress === 0x7000
                        ? 70 + phase
                    : 50 + phase;
        const view = new DataView(instance.exports.memory.buffer);
        if (numericAddress === 0x6000) {
            if (size !== 12) throw new Error(`unexpected record read size: ${size}`);
            view.setFloat32(destination, 1 + phase, true);
            view.setFloat32(destination + 4, 2 + phase, true);
            view.setFloat32(destination + 8, 3 + phase, true);
            return 1;
        }
        if (size !== 4) throw new Error(`unexpected scalar read size: ${size}`);
        view.setInt32(destination, value, true);
        return 1;
    },
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

instance.exports.update();
phase = 1;
instance.exports.update();
phase = 2;
instance.exports.update();
phase = 3;
instance.exports.update();

const expected = [
    // Phase zero initializes both snapshots and remains silent. In phase one,
    // failed required fields retain their accepted values while successful
    // siblings advance. The optional read deliberately accepts None instead.
    "10,20,90,1,2,3,70->11,20,90,2,3,4,71:-1",
    "11,20,90,2,3,4,71->12,22,94,3,4,5,None:-1",
    "12,22,94,3,4,5,None->13,23,96,4,5,6,73:-1",
];
if (JSON.stringify(snapshots) !== JSON.stringify(expected)) {
    throw new Error(`unexpected transactional snapshots: ${JSON.stringify({ expected, snapshots })}`);
}

console.log(JSON.stringify({ snapshots }));
