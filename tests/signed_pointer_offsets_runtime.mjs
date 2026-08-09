import { SplitScriptHost } from "./support/splitscript_host.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/signed_pointer_offsets_runtime.mjs <offsets.wasm>");
}

const pointers = new Map([
    [0x2000n, 0x2110n],
    [0x2fe0n, 0x3100n],
    [0x3110n, 0x3200n],
    [0x3fe0n, 0x4100n],
]);
const values = new Map([
    [0x1000n, 11],
    [0x2100n, 22],
    [0xfffffffffffffff0n, 33],
]);

const host = await SplitScriptHost.instantiate(wasmPath);
host.addProcess("game.exe", {
    modules: {
        "game.dll": { address: 0x1020n, size: 0x1000n },
    },
    read({ address, outputPointer, length, host: attachedHost }) {
        address = BigInt.asUintN(64, address);
        if (pointers.has(address)) {
            if (length !== 8) {
                throw new Error(`unexpected pointer width at ${address.toString(16)}`);
            }
            attachedHost.view().setBigUint64(outputPointer, pointers.get(address), true);
            return true;
        }
        if (values.has(address)) {
            if (length !== 4) {
                throw new Error(`unexpected value width at ${address.toString(16)}`);
            }
            attachedHost.view().setInt32(outputPointer, values.get(address), true);
            return true;
        }
        return false;
    },
});
host.start();
host.updateUntil(
    () => host.variables.has("Signed Pointer Offsets"),
    "the script never published its pointer results",
);

const observed = host.variables.get("Signed Pointer Offsets");
const expected = "11,22,12800,16624,33";
if (observed !== expected) {
    throw new Error(`unexpected signed-pointer output: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify({ observed }));
