import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/timer_split_index_runtime.mjs <split-index.wasm>");
}

const bytes = fs.readFileSync(wasmPath);
const decoder = new TextDecoder();
const hostIndices = [-1n, -27n, -(1n << 63n), 0n, 42n];
const observed = [];
let instance;
let index = 0;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 0,
    timer_current_split_index() {
        const value = hostIndices[Math.min(index, hostIndices.length - 1)];
        index += 1;
        return value;
    },
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        if (text(keyPointer, keyLength) === "Split Index") {
            observed.push(text(valuePointer, valueLength));
        }
    },
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
};

({ instance } = await WebAssembly.instantiate(bytes, { env }));
instance.exports._start();
for (let tick = 0; tick <= hostIndices.length; tick += 1) {
    instance.exports.update();
}

const expected = ["None", "None", "None", "0", "42"];
if (JSON.stringify(observed) !== JSON.stringify(expected)) {
    throw new Error(`unexpected split indices: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify({ observed }));
