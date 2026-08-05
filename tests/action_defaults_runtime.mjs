import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) throw new Error("usage: node tests/action_defaults_runtime.mjs <actions.wasm>");

const bytes = fs.readFileSync(wasmPath);
const decoder = new TextDecoder();
let instance;
let timerState = 0;
const calls = {
    start: 0,
    split: 0,
    reset: 0,
    pause: 0,
    resume: 0,
    gameTime: 0,
    print: 0,
    tickRate: 0,
};

const env = {
    timer_get_state: () => timerState,
    timer_start() { calls.start += 1; },
    timer_split() { calls.split += 1; },
    timer_reset() { calls.reset += 1; },
    timer_set_game_time() { calls.gameTime += 1; },
    timer_pause_game_time() { calls.pause += 1; },
    timer_resume_game_time() { calls.resume += 1; },
    timer_set_variable() {},
    runtime_set_tick_rate(rate) {
        if (rate !== 42) throw new Error(`unexpected setup tick rate: ${rate}`);
        calls.tickRate += 1;
    },
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    process_read: () => 1,
    process_get_module_address: () => 0n,
    process_get_module_size: () => 0n,
    runtime_print_message(pointer, length) {
        const message = decoder.decode(
            new Uint8Array(instance.exports.memory.buffer, pointer, length),
        );
        if (message !== "true") throw new Error(`setup observed stale settings: ${message}`);
        calls.print += 1;
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

if (calls.print !== 1 || calls.tickRate !== 1) {
    throw new Error(`setup did not run exactly once during _start: ${JSON.stringify(calls)}`);
}

// A NotRunning tick evaluates start; its fallthrough must be false.
instance.exports.update();

// A Running tick evaluates all other timer actions. Their fallthroughs must
// leave pause/game-time state untouched and must not reset or split.
timerState = 1;
instance.exports.update();

// The next tick takes the explicit-None branches, which have identical host
// behavior to the nullable blocks falling through.
instance.exports.update();

if (calls.start || calls.split || calls.reset || calls.pause || calls.resume || calls.gameTime) {
    throw new Error(`action fallthrough caused host calls: ${JSON.stringify(calls)}`);
}
if (calls.print !== 1 || calls.tickRate !== 1) {
    throw new Error(`setup ran again during update: ${JSON.stringify(calls)}`);
}

console.log(JSON.stringify(calls));
