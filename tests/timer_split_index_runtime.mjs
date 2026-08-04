import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/timer_split_index_runtime.mjs <split-index.wasm>");
}

const bytes = fs.readFileSync(wasmPath);
const decoder = new TextDecoder();
const hostIndices = [-1n, -27n, -(1n << 63n), 0n, 42n];
const observed = [];
const observedHistory = [];
let skips = 0;
let undos = 0;
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
    timer_segment_splitted(segmentIndex) {
        if (segmentIndex === 0n) return 1;
        if (segmentIndex === 1n) return 0;
        return -1;
    },
    timer_skip_split() { skips += 1; },
    timer_undo_split() { undos += 1; },
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        const key = text(keyPointer, keyLength);
        if (key === "Split Index") {
            observed.push(text(valuePointer, valueLength));
        } else if (key === "Split History") {
            observedHistory.push(text(valuePointer, valueLength));
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
if (observedHistory.some((value) => value !== "true,false,None")) {
    throw new Error(`unexpected segment history: ${JSON.stringify(observedHistory)}`);
}
if (skips !== 1 || undos !== 1) {
    throw new Error(`timer history mutations differed: ${JSON.stringify({ skips, undos })}`);
}

console.log(JSON.stringify({ observed, observedHistory, skips, undos }));
