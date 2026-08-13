import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/instant_helpers_runtime.mjs <instant-helpers.wasm>");
}

const decoder = new TextDecoder();
const readings = [
    1_000_000_000n,
    1_250_000_000n,
    9_223_372_036_854_775_707n,
    1_500_000_000n,
];
let instance;
let helperStatus;
let clockCalls = 0;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 1,
    runtime_set_tick_rate() {},
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        if (text(keyPointer, keyLength) === "Instant Helpers") {
            helperStatus = text(valuePointer, valueLength);
        }
    },
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
};

const wasi = {
    clock_time_get(clockId, precision, destination) {
        if (clockId !== 1 || precision !== 1n || clockCalls >= readings.length) {
            return 28;
        }
        new DataView(instance.exports.memory.buffer).setBigUint64(
            destination,
            readings[clockCalls],
            true,
        );
        clockCalls += 1;
        return 0;
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), {
    env,
    wasi_snapshot_preview1: wasi,
}));
instance.exports._start();
instance.exports.update(); // Initialize old and current.
instance.exports.update();
instance.exports.update();

if (helperStatus !== "ok" || clockCalls !== readings.length) {
    throw new Error(`unexpected instant-helper output: ${JSON.stringify({ helperStatus, clockCalls })}`);
}

console.log(JSON.stringify({ helperStatus, clockCalls }));
