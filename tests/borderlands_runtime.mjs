import fs from "node:fs";

const wasmPath = process.argv[2];
const mode = process.argv[3] ?? "v100";
if (!wasmPath) {
    throw new Error("usage: node tests/borderlands_runtime.mjs <borderlands.wasm> [v100|v150|unknown]");
}

const versions = {
    v100: [1, 0, 0, 0],
    v150: [1, 5, 0, 0],
    unknown: [9, 9, 9, 9],
};
const version = versions[mode] ?? versions.v100;
const moduleBase = 0x10000000n;
const firstPointer = 0x30000000n;
const secondPointer = 0x31000000n;
const decoder = new TextDecoder();
const stateReads = [];
const messages = [];
let instance;
let processOpen = true;
let isLoading = 1;
let pauses = 0;
let resumes = 0;
let detaches = 0;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

function writePeValue(address, destination, size) {
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
        view.setUint16(destination, 0x010b, true);
    } else if (address === moduleBase + 0x108n && size === 8) {
        view.setUint32(destination, 0x200, true);
        view.setUint32(destination + 4, 0x400, true);
    } else if (address === moduleBase + 0x200n && size === 16) {
        view.setUint16(destination + 14, 1, true);
    } else if (address === moduleBase + 0x210n && size === 4) {
        view.setUint32(destination, 0x10, true);
    } else if (address === moduleBase + 0x214n && size === 4) {
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
    } else if (address === moduleBase + 0x330n && size === 8) {
        const [major, minor, build, privatePart] = version;
        view.setUint16(destination, minor, true);
        view.setUint16(destination + 2, major, true);
        view.setUint16(destination + 4, privatePart, true);
        view.setUint16(destination + 6, build, true);
    } else {
        return false;
    }
    return true;
}

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
        return text(pointer, length) === "Borderlands.exe" ? 1n : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_get_module_address(_process, pointer, length) {
        if (text(pointer, length) !== "Borderlands.exe") {
            throw new Error("queried an unexpected module");
        }
        return moduleBase;
    },
    process_get_module_size: () => 0x04000000n,
    process_read(_process, address, destination, size) {
        if (writePeValue(address, destination, size)) {
            return 1;
        }

        const view = new DataView(instance.exports.memory.buffer);
        stateReads.push([address, size]);
        if (mode === "v100"
            && (address === moduleBase + 0x01b98a74n
                || address === moduleBase + 0x01b97e38n)
            && size === 1) {
            view.setUint8(destination, isLoading);
        } else if (mode === "v150"
            && address === moduleBase + 0x00480af0n
            && size === 4) {
            view.setUint32(destination, Number(firstPointer), true);
        } else if (mode === "v150"
            && address === moduleBase + 0x001e1f1cn
            && size === 4) {
            view.setUint32(destination, Number(secondPointer), true);
        } else if (mode === "v150"
            && (address === firstPointer || address === secondPointer)
            && size === 1) {
            view.setUint8(destination, isLoading);
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

if (mode === "unknown") {
    processOpen = false;
    instance.exports.update();
    if (stateReads.length !== 0 || pauses !== 0 || resumes !== 0 || detaches !== 1
        || messages.join(",") !== "unsupported Borderlands file version 9.9.9.9") {
        throw new Error(`unsupported build was not inert: ${JSON.stringify({ stateReads, pauses, resumes, detaches, messages })}`);
    }
} else {
    isLoading = 0;
    instance.exports.update();
    const expectedReads = mode === "v100" ? [
        [moduleBase + 0x01b98a74n, 1],
        [moduleBase + 0x01b97e38n, 1],
        [moduleBase + 0x01b98a74n, 1],
        [moduleBase + 0x01b97e38n, 1],
    ] : [
        [moduleBase + 0x00480af0n, 4],
        [firstPointer, 1],
        [moduleBase + 0x001e1f1cn, 4],
        [secondPointer, 1],
        [moduleBase + 0x00480af0n, 4],
        [firstPointer, 1],
        [moduleBase + 0x001e1f1cn, 4],
        [secondPointer, 1],
    ];
    if (stateReads.length !== expectedReads.length
        || stateReads.some(([address, size], index) =>
            address !== expectedReads[index][0] || size !== expectedReads[index][1])) {
        throw new Error(`layout selected the wrong reads: ${stateReads.map(([address, size]) => `${address.toString(16)}:${size}`)}`);
    }
    if (pauses !== 1 || resumes !== 1 || detaches !== 0 || messages.length !== 0) {
        throw new Error(`load removal behaved incorrectly: ${JSON.stringify({ pauses, resumes, detaches, messages })}`);
    }
}

console.log(JSON.stringify({ mode, stateReads: stateReads.length, pauses, resumes, detaches }));
