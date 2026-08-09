import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/tick_rate_lifecycle_runtime.mjs <tick-rate.wasm>");
}

const bytes = fs.readFileSync(wasmPath);
const decoder = new TextDecoder();
const tickRates = [];
const attachAttempts = [];
let attachedName = "hundred.exe";
let processOpen = true;
let detaches = 0;
let nextHandle = 1n;
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
    timer_set_variable() {},
    runtime_set_tick_rate(rate) { tickRates.push(rate); },
    runtime_print_message() {},
    process_attach(pointer, length) {
        const candidate = text(pointer, length);
        attachAttempts.push(candidate);
        return processOpen && candidate === attachedName ? nextHandle++ : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_read: () => 0,
    process_get_module_address: () => 0n,
    process_get_module_size: () => 0n,
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

const updateUntil = (condition, description) => {
    for (let tick = 0; tick < 16 && !condition(); tick += 1) {
        instance.exports.update();
    }
    if (!condition()) {
        throw new Error(`${description}: ${JSON.stringify({ tickRates, attachAttempts, detaches })}`);
    }
};

updateUntil(() => tickRates.includes(100), "the first process never selected 100 Hz");

processOpen = false;
updateUntil(
    () => tickRates.length >= 3 && tickRates.at(-1) === 60,
    "detaching did not restore the authored 60 Hz baseline",
);

attachedName = "hundred-twenty.exe";
processOpen = true;
updateUntil(() => tickRates.includes(120), "the second process never selected 120 Hz");

processOpen = false;
updateUntil(
    () => tickRates.length >= 5 && tickRates.at(-1) === 60,
    "the second detach did not restore the authored baseline",
);

const expectedRates = [60, 100, 60, 120, 60];
if (JSON.stringify(tickRates) !== JSON.stringify(expectedRates)) {
    throw new Error(`unexpected tick-rate transitions: ${JSON.stringify({ expectedRates, tickRates })}`);
}
if (detaches !== 2) {
    throw new Error(`unexpected detach count: ${detaches}`);
}

console.log(JSON.stringify({ tickRates, attachAttempts, detaches }));
