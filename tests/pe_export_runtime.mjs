import { SplitScriptHost } from "./support/splitscript_host.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/pe_export_runtime.mjs <autosplitter.wasm>");
}

const base = 0x1000n;
const image = new Uint8Array(0x1000);
const view = new DataView(image.buffer);
const writeU16 = (offset, value) => view.setUint16(offset, value, true);
const writeU32 = (offset, value) => view.setUint32(offset, value, true);

writeU16(0x00, 0x5a4d);
writeU32(0x3c, 0x80);
writeU32(0x80, 0x00004550);
writeU16(0x98, 0x020b);
writeU32(0x108, 0x200);
writeU32(0x10c, 0x100);
writeU32(0x214, 2);
writeU32(0x218, 2);
writeU32(0x21c, 0x300);
writeU32(0x220, 0x310);
writeU32(0x224, 0x320);
writeU32(0x300, 0x500);
writeU32(0x304, 0x260);
writeU32(0x310, 0x400);
writeU32(0x314, 0x420);
writeU16(0x320, 0);
writeU16(0x322, 1);
image.set(new TextEncoder().encode("mono_assembly_foreach\0"), 0x400);
image.set(new TextEncoder().encode("forwarded\0"), 0x420);

const host = await SplitScriptHost.instantiate(wasmPath);
host.addProcess("game.exe", {
    modules: {
        "mono-2.0-bdwgc.dll": {
            address: base,
            size: BigInt(image.length),
        },
    },
    ranges: [{ address: base, bytes: image, flags: 5n }],
});
host.start();
host.update();

const expected = [String(base + 0x500n), "missing", "rejected"];
if (JSON.stringify(host.messages) !== JSON.stringify(expected)) {
    throw new Error(`unexpected PE export result: ${host.json(host.summary())}`);
}

console.log(host.json({ messages: host.messages }));
