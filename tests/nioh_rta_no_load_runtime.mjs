import fs from "node:fs";

const wasmPath = process.argv[2];
const mode = process.argv[3] ?? "v12104";
if (!wasmPath || !["v12104", "v12105", "v12106", "unknown"].includes(mode)) {
    throw new Error("usage: node tests/nioh_rta_no_load_runtime.mjs <nioh.wasm> [v12104|v12105|v12106|unknown]");
}

const layouts = {
    v12104: {
        size: 63_766_528n,
        onMap: { root: 0x2171030n },
        missionTimer: { root: 0x20d8588n, offset: 0x1e0aacn },
        version: "1.21.04",
    },
    v12105: {
        size: 63_827_968n,
        onMap: { root: 0x2176f80n, offset: 0x90n },
        missionTimer: { root: 0x20a7418n, offset: 0x5d8n },
        version: "1.21.05",
    },
    v12106: {
        size: 63_848_448n,
        onMap: { root: 0x2176f80n, offset: 0x90n },
        missionTimer: { root: 0x20a7418n, offset: 0x5d8n },
        version: "1.21.06",
    },
};
const unknown = mode === "unknown";
const selected = layouts[mode] ?? layouts.v12104;
const moduleBase = 0x400000n;
const onMapPointer = 0x30000000n;
const missionTimerPointer = 0x40000000n;
const onMapAddress = selected.onMap.offset === undefined
    ? selected.onMap.root
    : onMapPointer + selected.onMap.offset;
const missionTimerAddress = missionTimerPointer + selected.missionTimer.offset;
const decoder = new TextDecoder();
const processNames = [];
const reads = [];
const messages = [];
const tickRates = [];
let instance;
let processOpen = true;
let onMap = 1;
let missionTimer = 10;
let failMissionTimerRead = false;
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
    runtime_set_tick_rate(rate) { tickRates.push(rate); },
    runtime_print_message(pointer, length) { messages.push(text(pointer, length)); },
    process_attach(pointer, length) {
        const name = text(pointer, length);
        processNames.push(name);
        return name === "Nioh.exe" ? 1n : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_get_module_address: () => moduleBase,
    process_get_module_size: () => unknown ? 1n : selected.size,
    process_read(_process, address, destination, size) {
        reads.push([address, size]);
        const view = new DataView(instance.exports.memory.buffer);
        if (selected.onMap.offset !== undefined
            && address === selected.onMap.root
            && size === 8) {
            view.setBigUint64(destination, onMapPointer, true);
        } else if (address === selected.missionTimer.root && size === 8) {
            view.setBigUint64(destination, missionTimerPointer, true);
        } else if (address === onMapAddress && size === 1) {
            view.setUint8(destination, onMap);
        } else if (address === missionTimerAddress && size === 4) {
            if (failMissionTimerRead) return 0;
            view.setFloat32(destination, missionTimer, true);
        } else {
            throw new Error(`unexpected Nioh read ${address.toString(16)} (${size}) in ${mode}`);
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

if (processNames.join(",") !== "Nioh.exe") {
    throw new Error(`ASL process-name migration differed: ${JSON.stringify(processNames)}`);
}

if (unknown) {
    if (reads.length !== 0 || tickRates.join(",") !== "1,29"
        || messages.join(",") !== "unsupported Nioh module size 1") {
        throw new Error(`unsupported build was not inert: ${JSON.stringify({ reads, tickRates, messages })}`);
    }
    processOpen = false;
    instance.exports.update();
    if (detaches !== 1 || tickRates.join(",") !== "1,29,1") {
        throw new Error(`unsupported detach differed: ${JSON.stringify({ detaches, tickRates })}`);
    }
} else {
    const expectedMessage = `Nioh ${selected.version} detected, module size ${selected.size}`;
    if (messages.join(",") !== expectedMessage || tickRates.join(",") !== "1,29") {
        throw new Error(`attach metadata differed: ${JSON.stringify({ messages, tickRates })}`);
    }

    onMap = 0;
    instance.exports.update();
    failMissionTimerRead = true;
    missionTimer = 11;
    onMap = 1;
    instance.exports.update();
    failMissionTimerRead = false;
    onMap = 0;
    instance.exports.update();
    instance.exports.update();

    const pointerReads = reads.filter(([, size]) => size === 8);
    if (pointerReads.some(([, size]) => size !== 8) || pointerReads.length === 0) {
        throw new Error(`pointer traversal was not 64-bit: ${JSON.stringify(pointerReads)}`);
    }
    if (pauses !== 2 || resumes !== 2) {
        throw new Error(`loading/failed-read behavior differed: ${JSON.stringify({ pauses, resumes })}`);
    }

    processOpen = false;
    instance.exports.update();
    if (detaches !== 1 || tickRates.join(",") !== "1,29,1") {
        throw new Error(`detach cadence differed: ${JSON.stringify({ detaches, tickRates })}`);
    }
}

console.log(JSON.stringify({ mode, reads: reads.length, pauses, resumes, detaches, tickRates }));
