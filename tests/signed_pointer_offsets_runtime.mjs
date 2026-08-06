import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/signed_pointer_offsets_runtime.mjs <offsets.wasm>");
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
    process_get_module_address(_process, pointer, length) {
        if (text(pointer, length) !== "game.dll") {
            throw new Error("unexpected module query");
        }
        return 0x1020n;
    },
    process_read(_process, address, destination, size) {
        address = BigInt.asUintN(64, address);
        const view = new DataView(instance.exports.memory.buffer);
        const pointers = new Map([
            [0x2000n, 0x2110n],
            [0x2fe0n, 0x3100n],
            [0x3110n, 0x3200n],
            [0x3fe0n, 0x4100n],
        ]);
        if (pointers.has(address)) {
            if (size !== 8) throw new Error(`unexpected pointer width at ${address.toString(16)}`);
            view.setBigUint64(destination, pointers.get(address), true);
            return 1;
        }
        const values = new Map([
            [0x1000n, 11],
            [0x2100n, 22],
            [0xfffffffffffffff0n, 33],
        ]);
        if (values.has(address)) {
            if (size !== 4) throw new Error(`unexpected value width at ${address.toString(16)}`);
            view.setInt32(destination, values.get(address), true);
            return 1;
        }
        return 0;
    },
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        if (text(keyPointer, keyLength) === "Signed Pointer Offsets") {
            observed = text(valuePointer, valueLength);
        }
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
for (let tick = 0; tick < 3 && observed === undefined; tick += 1) {
    instance.exports.update();
}

const expected = "11,22,12800,16624,33";
if (observed !== expected) {
    throw new Error(`unexpected signed-pointer output: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify({ observed }));
