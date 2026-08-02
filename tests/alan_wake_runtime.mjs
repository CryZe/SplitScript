import fs from "node:fs";

const wasmPath = process.argv[2];
const mode = process.argv[3] ?? "steam";
if (!wasmPath) {
    throw new Error("usage: node tests/alan_wake_runtime.mjs <alan-wake.wasm> [steam|gog|epic|unknown]");
}

const layouts = {
    steam: {
        size: 3_805_184n,
        build: "Steam",
        loading: 0x36ba34n,
        level: [0x36d8b4n, 0x3f0n, 0x174n],
        video: [0x2c0934n, 0x5c8n],
    },
    gog: {
        size: 3_809_280n,
        build: "GOG",
        loading: 0x36ca74n,
        level: [0x36e618n, 0x208n],
        video: [0x2c1974n, 0x5c8n],
    },
    epic: {
        size: 3_801_088n,
        build: "Epic",
        loading: 0x36aa74n,
        level: [0x36c618n, 0x208n],
        video: [0x2bf974n, 0x5c8n],
    },
};
const unknown = mode === "unknown";
const layout = layouts[mode] ?? layouts.steam;
const moduleSize = unknown ? 1n : layout.size;
const moduleBase = 0x10000000n;
const firstLevelPointer = 0x20000000n;
const finalLevelPointer = 0x30000000n;
const videoPointer = 0x40000000n;
const decoder = new TextDecoder();
const moduleNames = [];
const messages = [];
const variables = new Map();
const pointerReadWidths = [];
let instance;
let level = 0;
let loading = 0;
let video = 17;
let splits = 0;
let processOpen = true;
let detaches = 0;
let stateReads = 0;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

function write(destination, size, value, signed = false) {
    const view = new DataView(instance.exports.memory.buffer);
    if (size === 1) {
        signed ? view.setInt8(destination, Number(value)) : view.setUint8(destination, Number(value));
    } else if (size === 2) {
        signed ? view.setInt16(destination, Number(value), true) : view.setUint16(destination, Number(value), true);
    } else if (size === 4) {
        signed ? view.setInt32(destination, Number(value), true) : view.setUint32(destination, Number(value), true);
    } else {
        throw new Error(`unexpected write width ${size}`);
    }
}

const env = {
    timer_get_state: () => 1,
    timer_start() {},
    timer_split() { splits += 1; },
    timer_reset() {},
    timer_set_game_time() {},
    timer_pause_game_time() {},
    timer_resume_game_time() {},
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        variables.set(text(keyPointer, keyLength), text(valuePointer, valueLength));
    },
    runtime_set_tick_rate() {},
    process_attach(pointer, length) {
        return text(pointer, length) === "AlanWake.exe" ? 1n : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_get_module_address(_process, pointer, length) {
        moduleNames.push(text(pointer, length));
        return moduleBase;
    },
    process_get_module_size(_process, pointer, length) {
        moduleNames.push(text(pointer, length));
        return moduleSize;
    },
    process_read(_process, address, destination, size) {
        if (address === moduleBase && size === 2) {
            write(destination, size, 0x5a4d);
        } else if (address === moduleBase + 0x3cn && size === 4) {
            write(destination, size, 0x80);
        } else if (address === moduleBase + 0x80n && size === 4) {
            write(destination, size, 0x00004550);
        } else if (address === moduleBase + 0x98n && size === 2) {
            write(destination, size, 0x010b);
        } else if (!unknown && address === moduleBase + layout.loading && size === 1) {
            stateReads += 1;
            write(destination, size, loading);
        } else if (!unknown && address === moduleBase + layout.level[0] && size === 4) {
            stateReads += 1;
            pointerReadWidths.push(size);
            write(
                destination,
                size,
                layout.level.length === 3 ? firstLevelPointer : finalLevelPointer,
            );
        } else if (!unknown && layout.level.length === 3
            && address === firstLevelPointer + layout.level[1] && size === 4) {
            stateReads += 1;
            pointerReadWidths.push(size);
            write(destination, size, finalLevelPointer);
        } else if (!unknown && address === finalLevelPointer + layout.level.at(-1) && size === 1) {
            stateReads += 1;
            write(destination, size, level);
        } else if (!unknown && address === moduleBase + layout.video[0] && size === 4) {
            stateReads += 1;
            pointerReadWidths.push(size);
            write(destination, size, videoPointer);
        } else if (!unknown && address === videoPointer + layout.video[1] && size === 4) {
            stateReads += 1;
            write(destination, size, video, true);
        } else {
            throw new Error(`unexpected process read ${address.toString(16)} (${size}) in ${mode}`);
        }
        return 1;
    },
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

instance.exports.update();
if (unknown) {
    processOpen = false;
    instance.exports.update();
} else {
    level = 1;
    loading = 1;
    video = 19;
    instance.exports.update();
}

if (moduleNames.join(",") !== "AlanWake.exe,AlanWake.exe") {
    throw new Error(`unexpected main-module queries: ${JSON.stringify(moduleNames)}`);
}
if (unknown) {
    if (stateReads !== 0 || splits !== 0 || detaches !== 1 || variables.has("Build")
        || messages.join(",") !== "unsupported Alan Wake module size 1") {
        throw new Error(`unsupported build lifetime was incorrect: ${JSON.stringify({ stateReads, splits, detaches, messages, variables: Object.fromEntries(variables) })}`);
    }
} else {
    if (variables.get("Build") !== layout.build || splits !== 1 || stateReads === 0) {
        throw new Error(`layout behavior was incorrect: ${JSON.stringify({ mode, splits, stateReads, variables: Object.fromEntries(variables) })}`);
    }
    if (pointerReadWidths.some((width) => width !== 4)) {
        throw new Error(`PE32 path used a non-32-bit pointer read: ${pointerReadWidths}`);
    }
}

console.log(JSON.stringify({ build: unknown ? "unsupported" : layout.build, splits, pointerReadWidths }));
