import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/wrapper_match_runtime.mjs <match.wasm>");
}

const bytes = fs.readFileSync(wasmPath);
const decoder = new TextDecoder();
let instance;
let observed;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 0,
    runtime_set_tick_rate() {},
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        if (text(keyPointer, keyLength) === "Wrapper Match") {
            observed = text(valuePointer, valueLength);
        }
    },
};

({ instance } = await WebAssembly.instantiate(bytes, { env }));
instance.exports._start();
for (let tick = 0; tick < 3 && observed === undefined; tick += 1) {
    instance.exports.update();
}

const expected = "none,7,large,error boom,9,3,4,true";
if (observed !== expected) {
    throw new Error(`unexpected wrapper match output: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify({ observed }));
