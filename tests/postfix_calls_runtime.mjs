import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/postfix_calls_runtime.mjs <postfix-calls.wasm>");
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
        if (Number(address) !== 0x100 || size !== 4) {
            throw new Error(`unexpected process read: ${address.toString(16)} (${size} bytes)`);
        }
        new DataView(instance.exports.memory.buffer).setUint32(destination, 77, true);
        return 1;
    },
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        if (text(keyPointer, keyLength) === "Postfix Calls") {
            observed = text(valuePointer, valueLength);
        }
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
instance.exports.update();

const expected = "42,1,42,42,42,77,1";
if (observed !== expected) {
    throw new Error(`unexpected postfix-call output: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify({ observed }));
