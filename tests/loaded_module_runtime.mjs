import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/loaded_module_runtime.mjs <autosplitter.wasm>");
}

const decoder = new TextDecoder();
const queries = [];
const messages = [];
let instance;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);
const moduleName = (pointer, length) => {
    const name = text(pointer, length);
    queries.push(name);
    return name;
};

const env = {
    timer_get_state: () => 0,
    runtime_set_tick_rate() {},
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    process_get_module_address(_process, pointer, length) {
        return moduleName(pointer, length) === "steam_api.dll" ? 0x12340000n : 0n;
    },
    process_get_module_size(_process, pointer, length) {
        return moduleName(pointer, length) === "steam_api.dll" ? 0x5000n : 0n;
    },
    runtime_print_message(pointer, length) {
        messages.push(text(pointer, length));
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
instance.exports.update();

const expectedMessages = ["305397760|20480", "missing"];
const expectedQueries = ["steam_api.dll", "steam_api.dll", "missing.dll"];
if (JSON.stringify(messages) !== JSON.stringify(expectedMessages)
    || JSON.stringify(queries) !== JSON.stringify(expectedQueries)) {
    throw new Error(`unexpected loaded-module probe: ${JSON.stringify({ messages, queries })}`);
}

console.log(JSON.stringify({ messages, queries }));
