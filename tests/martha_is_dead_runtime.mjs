import fs from "node:fs";

const wasmPath = process.argv[2];
const mode = process.argv[3] ?? "demo";
if (!wasmPath) {
    throw new Error("usage: node tests/martha_is_dead_runtime.mjs <martha-is-dead.wasm> [demo|v427|v1040101|unknown]");
}

const layouts = {
    demo: { size: 84_725_760n, pointerOffset: 0x04a693d8n, finalOffset: 0x4e0n },
    v427: { size: 104_284_160n, pointerOffset: 0x05cd4600n, finalOffset: 0x4f8n },
    v1040101: { size: 104_288_256n, pointerOffset: 0x05ce0a38n, finalOffset: 0x4e0n },
};
const unknown = mode === "unknown";
const selected = layouts[mode] ?? layouts.demo;
const moduleSize = unknown ? 1n : selected.size;
const moduleBase = 0x10000000n;
const loaderPointer = 0x20000000n;
const decoder = new TextDecoder();
const reads = [];
const messages = [];
let instance;
let processOpen = true;
let loader = 1;
let pauses = 0;
let resumes = 0;
let detaches = 0;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 1,
    timer_start() {},
    timer_split() {},
    timer_reset() {},
    timer_set_game_time() {},
    timer_pause_game_time() { pauses += 1; },
    timer_resume_game_time() { resumes += 1; },
    timer_set_variable() {},
    runtime_set_tick_rate() {},
    runtime_print_message(pointer, length) { messages.push(text(pointer, length)); },
    process_attach(pointer, length) {
        return text(pointer, length) === "MID-Win64-Shipping.exe" ? 1n : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_get_module_address(_process, pointer, length) {
        if (text(pointer, length) !== "MID-Win64-Shipping.exe") {
            throw new Error("queried an unexpected module");
        }
        return moduleBase;
    },
    process_get_module_size: () => moduleSize,
    process_read(_process, address, destination, size) {
        reads.push([address, size]);
        const view = new DataView(instance.exports.memory.buffer);
        if (!unknown && address === moduleBase + selected.pointerOffset && size === 8) {
            view.setBigUint64(destination, loaderPointer, true);
        } else if (!unknown && address === loaderPointer + selected.finalOffset && size === 4) {
            view.setInt32(destination, loader, true);
        } else {
            throw new Error(`unexpected process read ${address.toString(16)} (${size}) in ${mode}`);
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
    if (reads.length !== 0 || pauses !== 0 || resumes !== 0 || detaches !== 1
        || messages.join(",") !== "unsupported Martha Is Dead module size 1") {
        throw new Error(`unsupported build was not inert: ${JSON.stringify({ reads, pauses, resumes, detaches, messages })}`);
    }
} else {
    // The first complete state read initializes old and current without
    // running lifecycle actions. The next poll observes the initialized state.
    instance.exports.update();
    loader = 0;
    instance.exports.update();
    const expectedReads = [
        moduleBase + selected.pointerOffset,
        loaderPointer + selected.finalOffset,
        moduleBase + selected.pointerOffset,
        loaderPointer + selected.finalOffset,
        moduleBase + selected.pointerOffset,
        loaderPointer + selected.finalOffset,
    ];
    if (reads.length !== expectedReads.length
        || reads.some(([address], index) => address !== expectedReads[index])) {
        throw new Error(`layout selected the wrong pointer path: ${reads.map(([address, size]) => `${address.toString(16)}:${size}`)}`);
    }
    if (pauses !== 1 || resumes !== 1 || detaches !== 0 || messages.length !== 0) {
        throw new Error(`load removal behaved incorrectly: ${JSON.stringify({ pauses, resumes, detaches, messages })}`);
    }
}

console.log(JSON.stringify({ mode, reads: reads.length, pauses, resumes, detaches }));
