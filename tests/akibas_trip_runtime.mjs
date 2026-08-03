import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath)
  throw new Error("usage: node tests/akibas_trip_runtime.mjs <akibas-trip.wasm>");

const moduleBase = 0x10000000n;
const decoder = new TextDecoder();
const settingValues = new Map();
const valueHandles = new Map();
const registeredBooleanKeys = new Set();
let nextValueHandle = 1n;
let instance;
let splits = 0;
let missionCounter = 0;
let mapNumber = 0;
let positionY = 0;
let positionZ = 0;
let positionX = 0;
let creditsStart = 0;
let trainEnd = 0;

const text = (pointer, length) =>
  decoder.decode(new Uint8Array(instance.exports.memory.buffer, pointer, length));

const env = {
  timer_get_state: () => 1,
  timer_start() {},
  timer_split() { splits += 1; },
  timer_reset() {},
  timer_set_game_time() {},
  timer_pause_game_time() {},
  timer_resume_game_time() {},
  timer_set_variable() {},
  runtime_set_tick_rate() {},
  runtime_print_message() {},
  process_attach(pointer, length) {
    return text(pointer, length) === "AkibaUU" ? 1n : 0n;
  },
  process_detach() {},
  process_is_open: () => 1,
  process_get_module_address(_process, pointer, length) {
    if (text(pointer, length) !== "AkibaUU.exe")
      throw new Error("the port queried an unexpected module");
    return moduleBase;
  },
  process_get_module_size: () => 0n,
  process_read(_process, address, destination, size) {
    const view = new DataView(instance.exports.memory.buffer);
    const offset = address - moduleBase;
    if (size !== 4)
      throw new Error(`unexpected Akiba read size ${size}`);
    if (offset === 0x4f6cfcn) view.setInt32(destination, missionCounter, true);
    else if (offset === 0x5324dcn) view.setInt32(destination, mapNumber, true);
    else if (offset === 0x465fe4n) view.setFloat32(destination, positionY, true);
    else if (offset === 0x465fe0n) view.setFloat32(destination, positionZ, true);
    else if (offset === 0x465fe8n) view.setFloat32(destination, positionX, true);
    else if (offset === 0x481500n) view.setInt32(destination, creditsStart, true);
    else if (offset === 0x4aca64n) view.setInt32(destination, trainEnd, true);
    else throw new Error(`unexpected Akiba read offset 0x${offset.toString(16)}`);
    return 1;
  },
  user_settings_add_bool(keyPointer, keyLength, _labelPointer, _labelLength, defaultValue) {
    const key = text(keyPointer, keyLength);
    registeredBooleanKeys.add(key);
    if (!settingValues.has(key)) settingValues.set(key, defaultValue !== 0);
    return settingValues.get(key) ? 1 : 0;
  },
  user_settings_add_title() {},
  user_settings_add_choice() {},
  user_settings_add_choice_option: () => 0,
  user_settings_add_file_select() {},
  user_settings_add_file_select_name_filter() {},
  user_settings_add_file_select_mime_filter() {},
  user_settings_set_tooltip() {},
  settings_map_load: () => 1n,
  settings_map_free() {},
  settings_map_get(_map, keyPointer, keyLength) {
    const key = text(keyPointer, keyLength);
    if (!settingValues.has(key)) return 0n;
    const handle = nextValueHandle++;
    valueHandles.set(handle, settingValues.get(key));
    return handle;
  },
  setting_value_free(handle) { valueHandles.delete(handle); },
  setting_value_get_bool(handle, outputPointer) {
    const value = valueHandles.get(handle);
    if (typeof value !== "boolean") return 0;
    new DataView(instance.exports.memory.buffer).setUint8(outputPointer, value ? 1 : 0);
    return 1;
  },
  setting_value_get_string: () => 0,
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
instance.exports.update();

if (!registeredBooleanKeys.has("1") || registeredBooleanKeys.has("mission1"))
  throw new Error("mission settings were not registered with their exact string keys");

missionCounter = 1;
positionY = 1.729497;
positionZ = -2.678275;
positionX = 2.182908;
instance.exports.update();
if (splits !== 1) throw new Error("enabled mission 1 did not split");

settingValues.set("2", false);
missionCounter = 2;
positionY = 1.44997;
positionZ = -3.382344;
positionX = 6.325909;
instance.exports.update();
if (splits !== 1) throw new Error("disabled mission 2 split unexpectedly");

settingValues.set("2", true);
missionCounter = 3;
instance.exports.update();
if (splits !== 2) throw new Error("live mission-key changes were not observed");

settingValues.set("chapter1", false);
missionCounter = 4;
positionY = 1.615439;
positionZ = -7.485596;
positionX = 13.636044;
instance.exports.update();
if (splits !== 2) throw new Error("the disabled chapter group did not gate mission 3");

positionY = 0;
positionZ = 0;
positionX = 0;
mapNumber = 25;
instance.exports.update();
missionCounter = 0;
mapNumber = 3;
instance.exports.update();
if (splits !== 3) throw new Error("the prologue map transition did not split");

if (valueHandles.size !== 0) throw new Error("setting value handles leaked");

console.log(JSON.stringify({ splits, booleanSettings: registeredBooleanKeys.size }));
