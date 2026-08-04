import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/host_metadata_runtime.mjs <host-metadata.wasm>");
}

const bytes = fs.readFileSync(wasmPath);
const decoder = new TextDecoder();
const encoder = new TextEncoder();
let instance;
let observed;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

function provideText(value, pointer, lengthPointer) {
    const encoded = encoder.encode(value);
    const view = new DataView(instance.exports.memory.buffer);
    const capacity = view.getUint32(lengthPointer, true);
    view.setUint32(lengthPointer, encoded.length, true);
    if (capacity < encoded.length) return 0;
    new Uint8Array(instance.exports.memory.buffer, pointer, encoded.length).set(encoded);
    return 1;
}

const env = {
    timer_get_state: () => 0,
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        if (text(keyPointer, keyLength) === "Host Metadata") {
            observed = text(valuePointer, valueLength);
        }
    },
    process_attach: () => 7n,
    process_detach() {},
    process_is_open: () => 1,
    process_get_path(process, pointer, lengthPointer) {
        if (process !== 7n) throw new Error(`unexpected process handle ${process}`);
        return provideText("/mnt/c/Games/game.exe", pointer, lengthPointer);
    },
    runtime_get_os: (pointer, lengthPointer) => provideText("windows", pointer, lengthPointer),
    runtime_get_arch: (pointer, lengthPointer) => provideText("x86_64", pointer, lengthPointer),
};

({ instance } = await WebAssembly.instantiate(bytes, { env }));
instance.exports._start();
for (let tick = 0; tick < 3 && observed === undefined; tick += 1) {
    instance.exports.update();
}

const expected = "/mnt/c/Games/game.exe|windows|x86_64";
if (observed !== expected) {
    throw new Error(`unexpected host metadata: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify({ observed }));
