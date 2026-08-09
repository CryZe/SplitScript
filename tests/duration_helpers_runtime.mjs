import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/duration_helpers_runtime.mjs <duration-helpers.wasm>");
}

const decoder = new TextDecoder();
let instance;
const gameTimes = [];
let helperStatus;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 1,
    timer_set_game_time(seconds, nanoseconds) {
        gameTimes.push([Number(seconds), nanoseconds]);
    },
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        if (text(keyPointer, keyLength) === "Duration Helpers") {
            helperStatus = text(valuePointer, valueLength);
        }
    },
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
// One initialization poll followed by the eleven observable helper steps.
for (let index = 0; index < 12; index += 1) {
    instance.exports.update();
}

const expected = [
    [1, 250_000_000],
    [-2, 750_000_000],
    [1, 500_000_000],
    [2, 250_000_000],
    [-2, 499_000_000],
    [2, 250_000_000],
    [-2, 750_000_000],
    [90, 0],
    [4_500, 0],
    [129_600, 0],
    [-2, 749_999_900],
];
if (helperStatus !== "ok" || JSON.stringify(gameTimes) !== JSON.stringify(expected)) {
    throw new Error(`unexpected duration-helper output: ${JSON.stringify({ expected, gameTimes, helperStatus })}`);
}

console.log(JSON.stringify({ gameTimes, helperStatus }));
