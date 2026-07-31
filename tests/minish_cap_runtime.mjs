import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) throw new Error("usage: node tests/minish_cap_runtime.mjs <autosplitter.wasm>");
const backend = process.argv[3] ?? "mgba";
if (backend !== "mgba" && backend !== "vba") {
    throw new Error(`unknown test backend ${backend}`);
}

const bytes = fs.readFileSync(wasmPath);
const decoder = new TextDecoder();
const processMemory = new Uint8Array(0x600000);
const processView = new DataView(processMemory.buffer);
const hostBase = 0x100000n;
const moduleBase = 0x500000n;
const gbaMemory = processMemory.subarray(Number(hostBase), Number(hostBase) + 0x48000);
let activeBase = Number(hostBase);
const attachNames = [];
const variables = new Map();
const gameTimes = [];
let instance;
let timerState = 0;
let timerStarts = 0;
let timerSplits = 0;
let invalidReads = 0;
let pauseCalls = 0;

function gbaOffset(address) {
    if (address >= 0x02000000 && address < 0x02040000) return address - 0x02000000;
    if (address >= 0x03000000 && address < 0x03008000) {
        return 0x40000 + address - 0x03000000;
    }
    throw new Error(`invalid GBA address ${address.toString(16)}`);
}

function setU8(address, value) {
    processView.setUint8(activeBase + gbaOffset(address), value);
}
function setU16(address, value) {
    processView.setUint16(activeBase + gbaOffset(address), value, true);
}
function setI32(address, value) {
    processView.setInt32(activeBase + gbaOffset(address), value, true);
}

setI32(0x03001e4e, 24);
setI32(0x0300187a, 144);
setU16(0x0300100c, 100);
setU8(0x0200af03, 18);
setU16(0x0200af0e, 123);
setU8(0x0200af12, 2);
setU8(0x02002b44, 3);
setU16(0x02002b02, 45);
setU8(0x02002aec, 6);

if (backend === "vba") {
    // Exercise VBA's 32-bit signature layout. The signatures contain the
    // addresses of two pointer slots, which then contain the current RAM
    // bases.
    const ewramSignature = Number(moduleBase) + 0x100;
    const iwramSignature = Number(moduleBase) + 0x200;
    const ewramPointer = 0x200000;
    const iwramPointer = 0x200004;
    processMemory.set([0xa1, 0, 0, 0, 0, 0x81, 0, 0xff, 0xff, 0x03, 0x00], ewramSignature);
    processMemory.set([0xa1, 0, 0, 0, 0, 0x81, 0, 0xff, 0x7f, 0x00, 0x00], iwramSignature);
    processView.setUint32(ewramSignature + 1, ewramPointer, true);
    processView.setUint32(iwramSignature + 1, iwramPointer, true);
    processView.setUint32(ewramPointer, Number(hostBase), true);
    processView.setUint32(iwramPointer, Number(hostBase) + 0x40000, true);
}

const env = {
    timer_get_state: () => timerState,
    timer_start() {
        timerStarts += 1;
        timerState = 1;
    },
    timer_split() {
        timerSplits += 1;
    },
    timer_reset() {},
    timer_set_game_time(seconds, nanoseconds) {
        gameTimes.push([seconds, nanoseconds]);
    },
    timer_pause_game_time() {
        pauseCalls += 1;
    },
    timer_resume_game_time() {},
    timer_set_variable(namePointer, nameLength, valuePointer, valueLength) {
        const memory = instance.exports.memory.buffer;
        const name = decoder.decode(new Uint8Array(memory, namePointer, nameLength));
        const value = decoder.decode(new Uint8Array(memory, valuePointer, valueLength));
        variables.set(name, value);
    },
    runtime_set_tick_rate() {},
    runtime_print_message() {},
    process_attach(pointer, length) {
        const name = decoder.decode(
            new Uint8Array(instance.exports.memory.buffer, pointer, length),
        );
        attachNames.push(name);
        return name === (backend === "vba" ? "visualboyadvance-m.exe" : "mGBA.exe") ? 1n : 0n;
    },
    process_detach() {},
    process_is_open: () => 1,
    process_read(_process, address, pointer, length) {
        const offset = Number(address);
        if (offset < 0 || offset + length > processMemory.length) {
            invalidReads += 1;
            return 0;
        }
        new Uint8Array(instance.exports.memory.buffer, pointer, length).set(
            processMemory.subarray(offset, offset + length),
        );
        return 1;
    },
    process_get_module_address(_process, pointer, length) {
        const name = decoder.decode(new Uint8Array(instance.exports.memory.buffer, pointer, length));
        return backend === "vba" && name === "visualboyadvance-m.exe" ? moduleBase : 0n;
    },
    process_get_module_size(_process, pointer, length) {
        const name = decoder.decode(new Uint8Array(instance.exports.memory.buffer, pointer, length));
        return backend === "vba" && name === "visualboyadvance-m.exe" ? 0x1000n : 0n;
    },
    process_get_memory_range_count: () => backend === "mgba" ? 1n : 0n,
    process_get_memory_range_address: () => hostBase,
    process_get_memory_range_size: () => 0x48000n,
    process_get_memory_range_flags: () => 0x6n,
    user_settings_add_bool: () => 1,
    user_settings_add_title() {},
    user_settings_add_choice() {},
    user_settings_add_choice_option: () => 1,
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

// Attach and establish the initial watcher snapshot.
instance.exports.update();
instance.exports.update();
instance.exports.update();
setI32(0x0300187a, 145);
setU16(0x0300100c, 101);
instance.exports.update();
// Acquire Smith's Sword on the following frame.
if (backend === "vba") {
    const relocatedBase = 0x300000;
    processMemory.copyWithin(relocatedBase, Number(hostBase), Number(hostBase) + 0x48000);
    processView.setUint32(0x200000, relocatedBase, true);
    processView.setUint32(0x200004, relocatedBase + 0x40000, true);
    activeBase = relocatedBase;
}
setU8(0x02002b32, 1 << 2);
setU16(0x0300100c, 102);
instance.exports.update();

const expectedAttachNames = backend === "vba"
    ? "visualboyadvance-m.exe"
    : "visualboyadvance-m.exe,VisualBoyAdvance.exe,mGBA.exe";
if (attachNames.join(",") !== expectedAttachNames) {
    throw new Error(`unexpected emulator attachment order: ${JSON.stringify(attachNames)}`);
}
if (timerStarts !== 1 || timerSplits !== 1) {
    throw new Error(
        `expected one start and split, got starts=${timerStarts} splits=${timerSplits} ` +
            `variables=${JSON.stringify(Object.fromEntries(variables))} invalidReads=${invalidReads}`,
    );
}
if (invalidReads !== 0) throw new Error(`GBA translation produced ${invalidReads} invalid reads`);
if (variables.get("Hearts") !== "4½" || variables.get("Rupees") !== "123") {
    throw new Error(`unexpected timer variables: ${JSON.stringify(Object.fromEntries(variables))}`);
}
if (pauseCalls === 0 || gameTimes.length === 0) {
    throw new Error("load removal did not supply paused game time");
}

console.log(JSON.stringify({ attachNames, timerStarts, timerSplits, variables: Object.fromEntries(variables) }));
