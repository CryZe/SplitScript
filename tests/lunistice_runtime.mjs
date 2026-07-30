import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) throw new Error("usage: node tests/lunistice_runtime.mjs <lunistice.wasm>");

const bytes = fs.readFileSync(wasmPath);
const dlc = process.argv.includes("--dlc");
const decoder = new TextDecoder();
const base = 0x1000n;
const memoryImage = new Uint8Array(0x10000);
const view = new DataView(memoryImage.buffer);
const messages = [];
const variables = new Map();
const tickRates = [];
const gameTimes = [];
let starts = 0;
let splits = 0;
let resets = 0;
let pauses = 0;
let timerState = 0;
let variableWrites = 0;
let failReads = false;
let processOpen = true;
let detaches = 0;
const levelOrSceneReadWidths = new Set();
const levelTimeVectorReads = new Set();
let instance;

const absolute = (relative) => 0x1000n + BigInt(relative);
const pointer = (relative, value) => view.setBigUint64(relative, BigInt(value), true);
const string = (relative, value) => memoryImage.set(new TextEncoder().encode(`${value}\0`), relative);
const field = (table, index, nameAddress, offset) => {
    pointer(table + index * 0x20, nameAddress);
    view.setUint32(table + index * 0x20 + 0x18, offset, true);
};

// IL2CPP registration signatures and assembly/image graph.
memoryImage.set([0x75, 0x11, 0x48, 0x8b, 0x1d], 0x500);
view.setInt32(0x505, 0x2f7, true);
memoryImage.set([0x48, 0x3b, 0x1d], 0x509);
string(0x600, "global-metadata.dat");
memoryImage.set([0x48, 0x8d, 0x0d], 0x700);
view.setInt32(0x703, -0x107, true);
memoryImage.set([0x48, 0xc1, 0xe9], 0x720);
memoryImage.set([0x48, 0x89, 0x05], 0x740);
view.setInt32(0x743, 0x1b9, true);
pointer(0x800, 0x1810);
pointer(0x808, 0x1818);
pointer(0x810, 0x1a00);
pointer(0xa00, 0x1b00);
pointer(0xa18, 0x1c00);
string(0xc00, "Assembly-CSharp");
view.setUint32(0xb18, 2, true);
pointer(0xb28, 0x1d00);
view.setUint32(0xd00, 0, true);
pointer(0x900, 0x1d10);
pointer(0xd10, 0x3000);
pointer(0xd18, 0x3800);

// Class names and common empty namespace.
string(0xa000, "GameManager");
string(0xa020, "Timer");
string(0xa040, "");

// GameManager class, fields, static table, and singleton.
pointer(0x2010, 0xb000);
pointer(0x2018, 0xb040);
pointer(0x2058, 0);
pointer(0x2080, 0x5000);
pointer(0x20b8, 0x7000);
view.setUint16(0x2120, 5, true);
const gameFieldNames = [
    "<Instance>k__BackingField",
    dlc ? "<GameState>k__BackingField" : "gameState",
    "_points",
    "_deaths",
    dlc ? "_currentScene" : "currentLevel",
];
const gameFieldOffsets = [0x20, 0x30, 0x34, 0x38, 0x3c];
for (let index = 0; index < gameFieldNames.length; index += 1) {
    const relative = 0xa100 + index * 0x30;
    string(relative, gameFieldNames[index]);
    field(0x4000, index, absolute(relative), gameFieldOffsets[index]);
}
pointer(0x6020, 0xa000);

// Timer class, fields, static table, and singleton.
pointer(0x2810, 0xb020);
pointer(0x2818, 0xb040);
pointer(0x2858, 0);
pointer(0x2880, 0x5200);
pointer(0x28b8, 0x7200);
view.setUint16(0x2920, 5, true);
const timerFieldNames = ["<Instance>k__BackingField", "currentLevelTime", "currentLevelTimeVector", "timerStopped", "character"];
const timerFieldOffsets = [0x20, 0x30, 0x34, 0x40, 0x44];
for (let index = 0; index < timerFieldNames.length; index += 1) {
    const relative = 0xa300 + index * 0x30;
    string(relative, timerFieldNames[index]);
    field(0x4200, index, absolute(relative), timerFieldOffsets[index]);
}
pointer(0x6220, 0xa100);

const gameManager = 0x9000;
const timer = 0x9100;
const sceneObject = 0xc000;
const sceneName = "Shrine01";
view.setUint32(sceneObject + 0x10, sceneName.length, true);
for (let index = 0; index < sceneName.length; index += 1) {
    view.setUint16(sceneObject + 0x14 + index * 2, sceneName.charCodeAt(index), true);
}
const setGame = ({ state = 0, points = 12, deaths = 3, level = 0 } = {}) => {
    view.setInt32(gameManager + 0x30, state, true);
    view.setInt32(gameManager + 0x34, points, true);
    view.setInt32(gameManager + 0x38, deaths, true);
    if (dlc) pointer(gameManager + 0x3c, absolute(sceneObject));
    else view.setInt32(gameManager + 0x3c, level, true);
};
const setTimer = ({ levelTime = 0, minutes = 0, seconds = 0, hundredths = 0, stopped = true, character = 0 } = {}) => {
    view.setFloat32(timer + 0x30, levelTime, true);
    view.setFloat32(timer + 0x34, minutes, true);
    view.setFloat32(timer + 0x38, seconds, true);
    view.setFloat32(timer + 0x3c, hundredths, true);
    view.setUint8(timer + 0x40, stopped ? 1 : 0);
    view.setUint32(timer + 0x44, character, true);
};
setGame();
setTimer();

const text = (pointer, length) => decoder.decode(new Uint8Array(instance.exports.memory.buffer, pointer, length));
const env = {
    timer_get_state: () => timerState,
    timer_start() { starts += 1; timerState = 1; },
    timer_split() { splits += 1; },
    timer_reset() { resets += 1; timerState = 0; },
    timer_set_game_time(seconds, nanos) { gameTimes.push([Number(seconds), nanos]); },
    timer_pause_game_time() { pauses += 1; },
    timer_resume_game_time() {},
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        variableWrites += 1;
        variables.set(text(keyPointer, keyLength), text(valuePointer, valueLength));
    },
    runtime_set_tick_rate(rate) { tickRates.push(rate); },
    process_attach(pointer, length) {
        const expected = dlc ? "Lunistice-Demo.exe" : "Lunistice.exe";
        return processOpen && text(pointer, length) === expected ? 1n : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_read(_process, address, pointer, length) {
        if (address === absolute(gameManager + 0x3c)) levelOrSceneReadWidths.add(length);
        if (address >= absolute(timer + 0x34) && address < absolute(timer + 0x40)) {
            levelTimeVectorReads.add(`${address - absolute(timer + 0x34)}:${length}`);
        }
        if (failReads) return 0;
        const offset = Number(address - base);
        if (offset < 0 || offset + length > memoryImage.length) return 0;
        new Uint8Array(instance.exports.memory.buffer, pointer, length).set(memoryImage.subarray(offset, offset + length));
        return 1;
    },
    process_get_module_address: () => base,
    process_get_module_size: () => BigInt(memoryImage.length),
    runtime_print_message(pointer, length) { messages.push(text(pointer, length)); },
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
for (let tick = 0; tick < 40 && !messages.includes("Found Timer"); tick += 1) instance.exports.update();
if (!messages.includes("Found Timer")) throw new Error(`attachment did not finish: ${JSON.stringify(messages)}`);

// Prime snapshots, transition timerStopped to false, enter results, then roll
// the level clock over. The accumulated game time must become 10 + 1 seconds.
instance.exports.update();
setTimer({ stopped: false });
instance.exports.update();
setTimer({ stopped: false, levelTime: 10, seconds: 10, character: dlc ? 6 : 1 });
instance.exports.update();
setGame({ state: 6, level: 0 });
instance.exports.update();
setGame({ state: 0, level: 1 });
setTimer({ stopped: false, levelTime: 1, seconds: 1, character: dlc ? 6 : 1 });
instance.exports.update();

// A torn read must not rotate snapshots or run any actions/variable writes.
const writesBeforeFailure = variableWrites;
const timesBeforeFailure = gameTimes.length;
setGame({ state: 0, points: 99, level: 1 });
failReads = true;
instance.exports.update();
failReads = false;
if (variableWrites !== writesBeforeFailure || gameTimes.length !== timesBeforeFailure) {
    throw new Error("a failed state read still committed a watcher tick");
}
if (variables.get("Points") !== "12") throw new Error("failed snapshot leaked partial state");

// Simulate a runner manually starting LiveSplit after accumulated time exists.
// The NotRunning -> Running transition must clear accumulation, and a level
// clock rollback before the first completed level must reset the timer.
setGame({ state: 0, points: 12, level: 0 });
timerState = 0;
instance.exports.update();
timerState = 1;
setTimer({ stopped: false, levelTime: 2, seconds: 2, character: dlc ? 6 : 1 });
instance.exports.update();
if (gameTimes.at(-1)?.[0] !== 2) {
    throw new Error(`runner-started timing did not clear accumulation: ${JSON.stringify(gameTimes)}`);
}
setTimer({ stopped: false, levelTime: 5, seconds: 5, character: dlc ? 6 : 1 });
instance.exports.update();
setTimer({ stopped: false, levelTime: 1, seconds: 1, character: dlc ? 6 : 1 });
instance.exports.update();

// Base-game final-level -> credits is the second split condition. The DLC
// scene-based layout intentionally does not use this transition.
instance.exports.update();
timerState = 1;
setGame({ state: 0, points: 12, level: 13 });
instance.exports.update();
setGame({ state: 0, points: 12, level: 2 });
instance.exports.update();

if (starts !== 1) {
    throw new Error(`expected one start, got ${starts}; variables=${JSON.stringify(Object.fromEntries(variables))}; messages=${JSON.stringify(messages)}`);
}
const expectedSplits = dlc ? 1 : 2;
if (splits !== expectedSplits) throw new Error(`expected ${expectedSplits} splits, got ${splits}`);
if (resets !== 1) throw new Error(`expected one early-level reset, got ${resets}`);
if (pauses < 3) throw new Error(`expected game time to remain paused, got ${pauses} pauses`);
if (!gameTimes.some(([seconds, nanos]) => seconds === 11 && nanos === 0)) {
    throw new Error(`expected accumulated game time 11.0, got ${JSON.stringify(gameTimes)}`);
}
const expectedVariables = new Map([
    ["Points", "12"],
    ["Resets", "3"],
    ["Level Time", "0:01.00"],
    [dlc ? "Scene" : "Level", dlc ? "Shrine01" : "2-1"],
    ["Character", dlc ? "Cres" : "Toree"],
]);
for (const [key, value] of expectedVariables) {
    if (variables.get(key) !== value) throw new Error(`expected ${key}=${value}, got ${variables.get(key)}`);
}

// Values outside the four known host states are normalized to
// TimerState.Unknown rather than leaking an integer into language code.
timerState = 99;
instance.exports.update();

// Closing the process returns the runtime to the detached polling rate
// immediately, without waiting for another attachment attempt.
processOpen = false;
instance.exports.update();
if (detaches !== 1) throw new Error(`expected one process detach, got ${detaches}`);
instance.exports.update();
if (JSON.stringify(tickRates) !== JSON.stringify([1, 120, 1])) {
    throw new Error(`unexpected tick rates: ${JSON.stringify(tickRates)}`);
}

const expectedBindingMessage = dlc ? "Found GameManager (DLC Demo)" : "Found GameManager (Base Game)";
if (!messages.includes(expectedBindingMessage)) throw new Error(`wrong binding: ${JSON.stringify(messages)}`);

const expectedLevelOrSceneReadWidths = [dlc ? 8 : 4];
if (JSON.stringify([...levelOrSceneReadWidths]) !== JSON.stringify(expectedLevelOrSceneReadWidths)) {
    throw new Error(
        `inactive level/scene representation was read: ${JSON.stringify([...levelOrSceneReadWidths])}`,
    );
}
if (JSON.stringify([...levelTimeVectorReads]) !== JSON.stringify(["0:12"])) {
    throw new Error(`level time vector was not read atomically: ${JSON.stringify([...levelTimeVectorReads])}`);
}

console.log(JSON.stringify({ dlc, starts, splits, resets, pauses, gameTimes, variables: Object.fromEntries(variables), tickRates, messages, levelOrSceneReadWidths: [...levelOrSceneReadWidths], levelTimeVectorReads: [...levelTimeVectorReads] }));
