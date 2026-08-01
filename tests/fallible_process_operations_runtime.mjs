import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/fallible_process_operations_runtime.mjs <operations.wasm>");
}

const bytes = fs.readFileSync(wasmPath);
const decoder = new TextDecoder();
const encoder = new TextEncoder();
let instance;
let observed;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 0,
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    process_read(_process, address, destination, size) {
        const source = Number(address);
        const view = new DataView(instance.exports.memory.buffer);
        if (source === 0x1000 && size === 8) {
            view.setBigUint64(destination, 0x1111n, true);
            return 1;
        }
        if (source === 0x3000 && size === 4) {
            view.setInt32(destination, 0x10, true);
            return 1;
        }
        if (source === 0x5010 && size === 4) {
            view.setInt32(destination, 2, true);
            return 1;
        }
        if (source === 0x5014 && size === 4) {
            view.setUint16(destination, "H".charCodeAt(0), true);
            view.setUint16(destination + 2, "i".charCodeAt(0), true);
            return 1;
        }
        if (source === 0x7000 && size === 16) {
            const output = new Uint8Array(instance.exports.memory.buffer, destination, size);
            output.fill(0);
            output.set(encoder.encode("Café"));
            return 1;
        }
        if (source === 0x7100 && size === 16) {
            const output = new Uint8Array(instance.exports.memory.buffer, destination, size);
            output.fill(0);
            output.set([0xc0, 0xaf]);
            return 1;
        }
        return 0;
    },
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        if (text(keyPointer, keyLength) === "Fallible Process Operations") {
            observed = text(valuePointer, valueLength);
        }
    },
};

({ instance } = await WebAssembly.instantiate(bytes, { env }));
instance.exports._start();
for (let tick = 0; tick < 3 && observed === undefined; tick += 1) {
    instance.exports.update();
}

const expected = "4369,12308,Hi,error,error,error,Café,Café,error,error";
if (observed !== expected) {
    throw new Error(`unexpected process operation output: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify({ observed }));
