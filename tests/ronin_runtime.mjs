import fs from "node:fs";

const wasmPath = process.argv[2];
const mode = process.argv[3] ?? "v8";
if (!wasmPath) throw new Error("usage: node tests/ronin_runtime.mjs <ronin.wasm> [v8|v9|unknown]");

const layouts = {
    v8: {
        size: 5_668_864n,
        loading: [0x002f01ccn, 0x7dcn, 0xc0n],
        bike: [0x561758n, 0x140n],
        bikeSize: 2,
        bikeValue: 21_368,
    },
    v9: {
        size: 5_705_728n,
        loading: [0x002f7228n, 0x5cn, 0x418n, 0x5cn, 0xb4n],
        bike: [0x56a758n, 0xe4n],
        bikeSize: 2,
        bikeValue: 52_688,
    },
};
const unknown = mode === "unknown";
const selected = layouts[mode] ?? layouts.v8;
const moduleBase = 0x10000000n;
const decoder = new TextDecoder();
const reads = [];
const messages = [];
const pointerTargets = new Map();
let nextPointer = 0x20000000n;
let instance;
let processOpen = true;
let timerState = 0;
let loading = 0;
let bike = 0;
let starts = 0;
let splits = 0;
let pauses = 0;
let detaches = 0;

function installPath(offsets, kind) {
    let address = moduleBase + offsets[0];
    for (const offset of offsets.slice(1, -1)) {
        const target = nextPointer;
        nextPointer += 0x10000n;
        pointerTargets.set(address, target);
        address = target + offset;
    }
    const target = nextPointer;
    nextPointer += 0x10000n;
    pointerTargets.set(address, target);
    return { address: target + offsets.at(-1), kind };
}

const loadingLeaf = installPath(selected.loading, "loading");
const bikeLeaf = installPath(selected.bike, "bike");
const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => timerState,
    timer_start() { starts += 1; timerState = 1; },
    timer_split() { splits += 1; },
    timer_reset() {},
    timer_set_game_time() {},
    timer_pause_game_time() { pauses += 1; },
    timer_resume_game_time() {},
    timer_set_variable() {},
    runtime_set_tick_rate() {},
    runtime_print_message(pointer, length) { messages.push(text(pointer, length)); },
    process_attach(pointer, length) { return text(pointer, length) === "Ronin.exe" ? 1n : 0n; },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_get_module_address: () => moduleBase,
    process_get_module_size: () => unknown ? 1n : selected.size,
    process_read(_process, address, destination, size) {
        reads.push([address, size]);
        const view = new DataView(instance.exports.memory.buffer);
        if (pointerTargets.has(address) && size === 8) {
            view.setBigUint64(destination, pointerTargets.get(address), true);
        } else if (address === loadingLeaf.address && size === 4) {
            view.setInt32(destination, loading, true);
        } else if (address === bikeLeaf.address && size === selected.bikeSize) {
            if (mode === "v8") view.setInt16(destination, bike, true);
            else view.setUint16(destination, bike, true);
        } else {
            throw new Error(`unexpected Ronin read ${address.toString(16)} (${size}) in ${mode}`);
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

if (unknown) {
    processOpen = false;
    instance.exports.update();
    if (reads.length !== 0 || starts !== 0 || splits !== 0 || detaches !== 1
        || messages.join(",") !== "unsupported Ronin module size 1") {
        throw new Error(`unsupported build was not inert: ${JSON.stringify({ reads, starts, splits, detaches, messages })}`);
    }
} else {
    loading = 1;
    instance.exports.update();
    loading = 0;
    bike = selected.bikeValue;
    instance.exports.update();
    if (starts !== 1 || pauses !== 1 || splits !== 1 || detaches !== 0 || messages.length !== 0) {
        throw new Error(`Ronin behavior was wrong: ${JSON.stringify({ starts, pauses, splits, detaches, messages })}`);
    }
}

console.log(JSON.stringify({ mode, reads: reads.length, starts, pauses, splits, detaches }));
