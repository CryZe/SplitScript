import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/cancellation_runtime.mjs <cancellation.wasm>");
}

const decoder = new TextDecoder();
const requests = [];
const detached = [];
const messages = [];
const secondPolls = new Map();
let retryPolls = 0;
let processOpen = true;
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
    runtime_set_tick_rate() {},
    process_attach() {
        return processOpen ? nextHandle++ : 0n;
    },
    process_detach(handle) {
        detached.push(handle);
    },
    process_is_open: () => processOpen ? 1 : 0,
    process_read(_handle, address, pointer, length) {
        if (address !== 0x3000n || length !== 4) return 0;
        retryPolls += 1;
        if (retryPolls < 3) return 0;
        new DataView(instance.exports.memory.buffer).setInt32(pointer, 42, true);
        return 1;
    },
    process_get_module_address(handle, pointer, length) {
        const name = text(pointer, length);
        requests.push([handle, name]);
        if (name === "First.dll") {
            return 0x1000n + handle;
        }
        const polls = (secondPolls.get(handle) ?? 0) + 1;
        secondPolls.set(handle, polls);
        return polls >= 2 ? 0x2000n + handle : 0n;
    },
    process_get_module_size: () => 0x100n,
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

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();

// Handle 1 completes the first await and suspends at the second.
instance.exports.update();
processOpen = false;
instance.exports.update();

// A fresh process must restart at the first await rather than resuming handle
// 1's continuation. The second await then succeeds on its next polling tick.
processOpen = true;
instance.exports.update();
instance.exports.update();

const pendingMessages = [
    "entered branch",
    "entered branch",
    "ready 4098:8194",
    "before next tick",
];
if (JSON.stringify(messages) !== JSON.stringify(pendingMessages)) {
    throw new Error(`nextTick did not suspend at the update boundary: ${JSON.stringify(messages)}`);
}
instance.exports.update();

// The retry expression is re-evaluated on subsequent updates without
// replaying the continuation that precedes it.
instance.exports.update();
const retryingMessages = [
    ...pendingMessages,
    "after next tick",
];
if (retryPolls !== 2 || JSON.stringify(messages) !== JSON.stringify(retryingMessages)) {
    throw new Error(`retry did not remain suspended: polls=${retryPolls}, messages=${JSON.stringify(messages)}`);
}
instance.exports.update();

const expectedRequests = [
    [1n, "First.dll"],
    [1n, "Second.dll"],
    [2n, "First.dll"],
    [2n, "Second.dll"],
    [2n, "Second.dll"],
];
if (JSON.stringify(requests, (_, value) => typeof value === "bigint" ? `${value}n` : value)
    !== JSON.stringify(expectedRequests, (_, value) => typeof value === "bigint" ? `${value}n` : value)) {
    throw new Error(`continuation was not restarted: ${JSON.stringify(requests, (_, value) => typeof value === "bigint" ? `${value}n` : value)}`);
}
if (detached.length !== 1 || detached[0] !== 1n) {
    throw new Error(`expected handle 1 to be detached once, got ${detached}`);
}
const expectedMessages = [
    "entered branch",
    "entered branch",
    "ready 4098:8194",
    "before next tick",
    "after next tick",
    "marker 42",
    "finished",
];
if (JSON.stringify(messages) !== JSON.stringify(expectedMessages)) {
    throw new Error(`unexpected completion messages: ${JSON.stringify(messages)}`);
}

console.log(JSON.stringify({ requests: requests.length, detached: detached.length, retryPolls, messages }));
