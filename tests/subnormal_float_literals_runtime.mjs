import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/subnormal_float_literals_runtime.mjs <literals.wasm>");
}

const decoder = new TextDecoder();
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
        const view = new DataView(instance.exports.memory.buffer);
        if (address === 0x100n && size === 4) {
            view.setUint32(destination, 1, true);
            return 1;
        }
        if (address === 0x108n && size === 8) {
            view.setBigUint64(destination, 1n, true);
            return 1;
        }
        throw new Error(`unexpected process read: ${address.toString(16)} (${size} bytes)`);
    },
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        if (text(keyPointer, keyLength) === "Subnormal Float Literals") {
            observed = text(valuePointer, valueLength);
        }
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
instance.exports.update();
instance.exports.update();

const expected = "true,true,true,true";
if (observed !== expected) {
    throw new Error(`unexpected subnormal output: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify({ observed }));
