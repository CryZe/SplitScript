import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) throw new Error("usage: node tests/dds_runtime.mjs <dds.wasm>");

const decoder = new TextDecoder();
const values = new Map();
const valueHandles = new Map();
const widgets = [];
const tooltips = new Map();
const pointers = [0x1000_0000n, 0x2000_0000n, 0x3000_0000n, 0x4000_0000n, 0x5000_0000n];
let nextValueHandle = 1n;
let instance;
let processOpen = true;
let level = 2;
let splits = 0;
let detaches = 0;

const text = (pointer, length) => decoder.decode(
  new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

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
    return text(pointer, length) === "DrugDealerSimulator-Win64-Shipping.exe" ? 1n : 0n;
  },
  process_detach() { detaches += 1; },
  process_is_open: () => processOpen ? 1 : 0,
  process_get_module_address: () => 0n,
  process_read(_process, address, destination, size) {
    const view = new DataView(instance.exports.memory.buffer);
    const pointerReads = [
      [0x02fd_8bb0n, pointers[0]],
      [pointers[0] + 0x20n, pointers[1]],
      [pointers[1] + 0xb0n, pointers[2]],
      [pointers[2] + 0x7d0n, pointers[3]],
      [pointers[3] + 0x350n, pointers[4]],
    ];
    const pointer = pointerReads.find(([candidate]) => candidate === address);
    if (pointer && size === 8) {
      view.setBigUint64(destination, pointer[1], true);
      return 1;
    }
    if (address === pointers[4] + 0x348n && size === 4) {
      view.setInt32(destination, level, true);
      return 1;
    }
    throw new Error(`unexpected DDS read ${address.toString(16)} (${size})`);
  },
  user_settings_add_bool(keyPointer, keyLength, labelPointer, labelLength, defaultValue) {
    const key = text(keyPointer, keyLength);
    const label = text(labelPointer, labelLength);
    if (!values.has(key)) values.set(key, defaultValue !== 0);
    widgets.push(["bool", key, label, defaultValue !== 0]);
    return values.get(key) ? 1 : 0;
  },
  user_settings_add_title(keyPointer, keyLength, labelPointer, labelLength, headingLevel) {
    widgets.push([
      "title",
      text(keyPointer, keyLength),
      text(labelPointer, labelLength),
      headingLevel,
    ]);
  },
  user_settings_add_choice() {},
  user_settings_add_choice_option: () => 0,
  user_settings_add_file_select() {},
  user_settings_add_file_select_name_filter() {},
  user_settings_add_file_select_mime_filter() {},
  user_settings_set_tooltip(keyPointer, keyLength, tooltipPointer, tooltipLength) {
    tooltips.set(text(keyPointer, keyLength), text(tooltipPointer, tooltipLength));
  },
  settings_map_load: () => 1n,
  settings_map_free() {},
  settings_map_get(_map, keyPointer, keyLength) {
    const key = text(keyPointer, keyLength);
    if (!values.has(key)) return 0n;
    const handle = nextValueHandle++;
    valueHandles.set(handle, values.get(key));
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

const booleanWidgets = widgets.filter(([kind]) => kind === "bool");
if (booleanWidgets.length !== 35) {
  throw new Error(`expected 35 generated settings, got ${booleanWidgets.length}`);
}
const expectedKeys = Array.from({ length: 35 }, (_, index) => String(index + 2));
if (JSON.stringify(booleanWidgets.map(([, key]) => key)) !== JSON.stringify(expectedKeys)) {
  throw new Error(`unexpected generated keys: ${JSON.stringify(booleanWidgets)}`);
}
if (booleanWidgets.some(([, key, label, enabled]) => key !== label || !enabled)) {
  throw new Error(`generated labels/defaults differed: ${JSON.stringify(booleanWidgets)}`);
}
if (widgets[0][0] !== "title" || widgets[0][2] !== "Levels" || widgets[0][3] !== 0) {
  throw new Error(`unexpected settings heading: ${JSON.stringify(widgets[0])}`);
}
if (tooltips.size !== 35 || [...tooltips.values()].some(
  tooltip => tooltip !== "Splits when the player reaches this level.",
)) {
  throw new Error(`unexpected generated tooltips: ${JSON.stringify([...tooltips])}`);
}

instance.exports.update();
instance.exports.update();
level = 3;
instance.exports.update();
if (splits !== 1) throw new Error(`enabled level did not split: ${splits}`);

values.set("4", false);
level = 4;
instance.exports.update();
if (splits !== 1) throw new Error(`disabled level split unexpectedly: ${splits}`);

level = 5;
instance.exports.update();
if (splits !== 2) throw new Error(`later enabled level did not split: ${splits}`);

processOpen = false;
instance.exports.update();
if (detaches !== 1) throw new Error(`detach behavior differed: ${detaches}`);

console.log(JSON.stringify({ settings: booleanWidgets.length, tooltips: tooltips.size, splits, detaches }));
