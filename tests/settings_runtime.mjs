import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath)
  throw new Error("usage: node tests/settings_runtime.mjs <settings.wasm>");

const bytes = fs.readFileSync(wasmPath);
const decoder = new TextDecoder();
const encoder = new TextEncoder();
const values = new Map();
const valueHandles = new Map();
const widgets = [];
const filters = [];
const tooltips = new Map();
const variables = new Map();
const messages = [];
let nextValueHandle = 10n;
let instance;

const text = (pointer, length) =>
  decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
  );
const optionalText = (pointer, length) =>
  pointer === 0 ? null : text(pointer, length);

const env = {
  timer_get_state: () => 0,
  timer_start() {},
  timer_split() {},
  timer_reset() {},
  timer_set_game_time() {},
  timer_pause_game_time() {},
  timer_resume_game_time() {},
  timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
    variables.set(text(keyPointer, keyLength), text(valuePointer, valueLength));
  },
  runtime_set_tick_rate() {},
  runtime_print_message(pointer, length) {
    messages.push(text(pointer, length));
  },
  process_attach(pointer, length) {
    return text(pointer, length) === "explorer.exe" ? 1n : 0n;
  },
  process_detach() {},
  process_is_open: () => 1,
  process_read: () => 1,
  process_get_module_address: () => 0n,
  process_get_module_size: () => 0n,
  user_settings_add_bool(
    keyPointer,
    keyLength,
    descriptionPointer,
    descriptionLength,
    defaultValue,
  ) {
    const key = text(keyPointer, keyLength);
    if (!values.has(key)) values.set(key, defaultValue !== 0);
    widgets.push(["bool", key, text(descriptionPointer, descriptionLength)]);
    return values.get(key) ? 1 : 0;
  },
  user_settings_add_title(
    keyPointer,
    keyLength,
    descriptionPointer,
    descriptionLength,
    level,
  ) {
    widgets.push([
      "title",
      text(keyPointer, keyLength),
      text(descriptionPointer, descriptionLength),
      level,
    ]);
  },
  user_settings_add_choice(
    keyPointer,
    keyLength,
    descriptionPointer,
    descriptionLength,
    defaultPointer,
    defaultLength,
  ) {
    const key = text(keyPointer, keyLength);
    if (!values.has(key)) values.set(key, text(defaultPointer, defaultLength));
    widgets.push(["choice", key, text(descriptionPointer, descriptionLength)]);
  },
  user_settings_add_choice_option(
    keyPointer,
    keyLength,
    optionPointer,
    optionLength,
    descriptionPointer,
    descriptionLength,
  ) {
    const key = text(keyPointer, keyLength);
    const option = text(optionPointer, optionLength);
    widgets.push([
      "option",
      key,
      option,
      text(descriptionPointer, descriptionLength),
    ]);
    return values.get(key) === option ? 1 : 0;
  },
  user_settings_add_file_select(
    keyPointer,
    keyLength,
    descriptionPointer,
    descriptionLength,
  ) {
    const key = text(keyPointer, keyLength);
    if (!values.has(key)) values.set(key, "");
    widgets.push(["file", key, text(descriptionPointer, descriptionLength)]);
  },
  user_settings_add_file_select_name_filter(
    keyPointer,
    keyLength,
    descriptionPointer,
    descriptionLength,
    patternPointer,
    patternLength,
  ) {
    filters.push([
      "name",
      text(keyPointer, keyLength),
      optionalText(descriptionPointer, descriptionLength),
      text(patternPointer, patternLength),
    ]);
  },
  user_settings_add_file_select_mime_filter(
    keyPointer,
    keyLength,
    mimePointer,
    mimeLength,
  ) {
    filters.push([
      "mime",
      text(keyPointer, keyLength),
      text(mimePointer, mimeLength),
    ]);
  },
  user_settings_set_tooltip(
    keyPointer,
    keyLength,
    tooltipPointer,
    tooltipLength,
  ) {
    tooltips.set(
      text(keyPointer, keyLength),
      text(tooltipPointer, tooltipLength),
    );
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
  setting_value_free(handle) {
    valueHandles.delete(handle);
  },
  setting_value_get_bool(handle, outputPointer) {
    const value = valueHandles.get(handle);
    if (typeof value !== "boolean") return 0;
    new DataView(instance.exports.memory.buffer).setUint8(
      outputPointer,
      value ? 1 : 0,
    );
    return 1;
  },
  setting_value_get_string(handle, outputPointer, lengthPointer) {
    const value = valueHandles.get(handle);
    const memory = new DataView(instance.exports.memory.buffer);
    if (typeof value !== "string") {
      memory.setUint32(lengthPointer, 0, true);
      return 0;
    }
    const encoded = encoder.encode(value);
    const capacity = memory.getUint32(lengthPointer, true);
    memory.setUint32(lengthPointer, encoded.length, true);
    if (capacity < encoded.length) return 0;
    new Uint8Array(
      instance.exports.memory.buffer,
      outputPointer,
      encoded.length,
    ).set(encoded);
    return 1;
  },
};

({ instance } = await WebAssembly.instantiate(bytes, { env }));
instance.exports._start();
instance.exports.update();

if (variables.get("Auto Splitting") !== "enabled")
  throw new Error("boolean default was not loaded");
if (variables.get("Capture Source") !== "Executable Name")
  throw new Error("choice default was not loaded");
if (variables.get("Layout File") !== "")
  throw new Error("file default was not loaded");

values.set("auto-splitting", false);
values.set("captureMode", "FullPath");
values.set("layoutFile", "/mnt/c/layout.json");
values.set("liveReload", false);
values.set("verboseLogging", true);
instance.exports.update();

if (variables.get("Auto Splitting") !== "disabled")
  throw new Error("live boolean change was not observed");
if (variables.get("Capture Source") !== "Full Path")
  throw new Error("live choice change was not observed");
if (variables.get("Layout File") !== "/mnt/c/layout.json")
  throw new Error("live file change was not observed");
if (!messages.includes("Live Reload is now disabled"))
  throw new Error(`oldSettings did not rotate: ${JSON.stringify(messages)}`);
if (!messages.includes("Verbose settings diagnostics tick"))
  throw new Error("live verbose setting was not observed");

const titles = widgets
  .filter(([kind]) => kind === "title")
  .map(([, , description, level]) => [description, level]);
const expectedTitles = [
  ["General", 0],
  ["Paths", 1],
  ["Advanced", 1],
  ["Diagnostics", 2],
];
if (JSON.stringify(titles) !== JSON.stringify(expectedTitles)) {
  throw new Error(`unexpected title hierarchy: ${JSON.stringify(titles)}`);
}
if (widgets.filter(([kind]) => kind === "option").length !== 3)
  throw new Error("choice options were not registered");
if (filters.length !== 5)
  throw new Error(`expected five file filters, got ${JSON.stringify(filters)}`);
if (tooltips.size !== 9)
  throw new Error(`expected nine tooltips, got ${tooltips.size}`);
if (
  tooltips.get("auto-splitting") !==
  "Turns the example split logic on or off without unloading the auto splitter."
)
  throw new Error("multiline documentation comments were not folded into a tooltip");
if (valueHandles.size !== 0) throw new Error("setting value handles leaked");

console.log(
  JSON.stringify({
    widgets,
    filters,
    tooltips: Object.fromEntries(tooltips),
    variables: Object.fromEntries(variables),
    messages,
  }),
);
