import fs from "node:fs";

const wasmPath = process.argv[2];
const mode = process.argv[3] ?? "steam";
if (!wasmPath) {
    throw new Error(
        "usage: node tests/a_plague_tale_innocence_runtime.mjs <autosplitter.wasm> [steam|epic|xbox|unknown]",
    );
}

const layouts = {
    steam: {
        size: 25_473_024n,
        playerOffset: 0x152e91cn,
        mapModule: "APlagueTaleInnocence_x64",
        mapOffsets: [0x015206e0n, 0x88n, 0x0n, 0xd0n, 0x990n, 0x260n],
        cutsceneOffset: 0x164bc34n,
    },
    epic: {
        size: 25_284_608n,
        playerOffset: 0x152e6dcn,
        mapModule: "APlagueTaleInnocence_x64",
        mapOffsets: [0x016aadc0n, 0x10n, 0x110n, 0xd8n, 0x10n, 0x30n, 0x170n, 0x260n],
        cutsceneOffset: 0x164b9f4n,
    },
    xbox: {
        size: 27_566_080n,
        playerOffset: 0x1744ebcn,
        mapModule: "MessageBus.dll",
        mapOffsets: [0x005c0de0n, 0x340n, 0x668n],
        cutsceneOffset: undefined,
    },
};

const unknown = mode === "unknown";
const selected = layouts[mode] ?? layouts.steam;
const executableBase = 0x10000000n;
const wwiseBase = 0x20000000n;
const messageBusBase = 0x30000000n;
const pointerBase = 0x40000000n;
const decoder = new TextDecoder();
const encoder = new TextEncoder();
const messages = [];
const reads = [];
let instance;
let processOpen = true;
let timerState = 0;
let playerControl = 4025;
let mapName = "WORLD>DOMAIN";
let cutsceneState = 0;
let loading = true;
let starts = 0;
let splits = 0;
let pauses = 0;
let resumes = 0;
let detaches = 0;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const moduleBase = (name) => {
    if (name === "APlagueTaleInnocence_x64") {
        return executableBase;
    }
    if (name === "WwiseLibPCx64R.dll") {
        return wwiseBase;
    }
    if (name === "MessageBus.dll") {
        return messageBusBase;
    }
    throw new Error(`queried unexpected module ${JSON.stringify(name)}`);
};

const mapPointerReads = new Map();
let pointer = pointerBase;
let pointerAddress = moduleBase(selected.mapModule) + selected.mapOffsets[0];
for (let index = 1; index < selected.mapOffsets.length; index += 1) {
    mapPointerReads.set(pointerAddress, pointer);
    pointerAddress = pointer + selected.mapOffsets[index];
    pointer += 0x01000000n;
}
const mapAddress = pointerAddress;

const env = {
    timer_get_state: () => timerState,
    timer_start() {
        starts += 1;
        timerState = 1;
    },
    timer_split() { splits += 1; },
    timer_reset() {},
    timer_set_game_time() {},
    timer_pause_game_time() { pauses += 1; },
    timer_resume_game_time() { resumes += 1; },
    timer_set_variable() {},
    runtime_set_tick_rate() {},
    runtime_print_message(pointer_, length) {
        messages.push(text(pointer_, length));
    },
    process_attach(pointer_, length) {
        return text(pointer_, length) === "APlagueTaleInnocence_x64" ? 1n : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_get_module_address(_process, pointer_, length) {
        return moduleBase(text(pointer_, length));
    },
    process_get_module_size: () => unknown ? 1n : selected.size,
    process_read(_process, address, destination, size) {
        reads.push([address, size]);
        const view = new DataView(instance.exports.memory.buffer);

        if (address === wwiseBase + 0x262521n && size === 1) {
            view.setUint8(destination, loading ? 1 : 0);
            return 1;
        }
        if (address === executableBase + selected.playerOffset && size === 4) {
            view.setInt32(destination, playerControl, true);
            return 1;
        }
        if (selected.cutsceneOffset !== undefined
            && address === executableBase + selected.cutsceneOffset
            && size === 4) {
            view.setInt32(destination, cutsceneState, true);
            return 1;
        }
        if (mapPointerReads.has(address) && size === 8) {
            view.setBigUint64(destination, mapPointerReads.get(address), true);
            return 1;
        }
        if (address === mapAddress && size === 50) {
            const output = new Uint8Array(instance.exports.memory.buffer, destination, size);
            output.fill(0);
            output.set(encoder.encode(mapName));
            return 1;
        }

        throw new Error(`unexpected process read ${address.toString(16)} (${size}) in ${mode}`);
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
    settings_map_get: () => 1n,
    setting_value_free() {},
    setting_value_get_bool(_handle, outputPointer) {
        new DataView(instance.exports.memory.buffer).setUint8(outputPointer, 1);
        return 1;
    },
    setting_value_get_string: () => 0,
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
instance.exports.update();

if (unknown) {
    processOpen = false;
    instance.exports.update();
    if (reads.length !== 0 || starts !== 0 || splits !== 0 || pauses !== 0 || detaches !== 1
        || messages.join(",") !== "unsupported A Plague Tale: Innocence module size 1") {
        throw new Error(
            `unsupported build was not inert: ${JSON.stringify({ reads, starts, splits, pauses, detaches, messages })}`,
        );
    }
} else {
    playerControl = 4024;
    loading = false;
    instance.exports.update();
    instance.exports.update();

    mapName = "WORLD>VILLAGE";
    instance.exports.update();
    const afterVillage = splits;
    loading = true;
    instance.exports.update();
    if (splits !== afterVillage) {
        throw new Error("the same chapter split more than once");
    }

    loading = false;
    mapName = "WORLD>VILLAGE2";
    instance.exports.update();
    mapName = "WORLD>EPILOGUE";
    instance.exports.update();
    cutsceneState = 1_079_474_040;
    instance.exports.update();

    const expectedSplits = mode === "xbox" ? 3 : 4;
    if (starts !== 1 || splits !== expectedSplits || pauses < 1 || resumes < 1
        || messages.length !== 0) {
        throw new Error(
            `autosplitter behavior differed: ${JSON.stringify({ mode, starts, splits, pauses, resumes, messages })}`,
        );
    }

    const pausesBeforeDetach = pauses;
    processOpen = false;
    instance.exports.update();
    if (detaches !== 1 || pauses !== pausesBeforeDetach + 1) {
        throw new Error(
            `process cleanup differed: ${JSON.stringify({ detaches, pausesBeforeDetach, pauses })}`,
        );
    }
}

console.log(JSON.stringify({ mode, reads: reads.length, starts, splits, pauses, resumes, detaches }));
