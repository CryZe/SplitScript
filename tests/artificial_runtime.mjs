import { SplitScriptHost } from "./support/splitscript_host.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/artificial_runtime.mjs <artificial.wasm>");
}

const moduleBase = 0x1000n;
const assemblyList = 0x8000n;
const link = 0x8100n;
const assembly = 0x8200n;
const assemblyName = 0x8300n;
const imageAddress = 0x8400n;
const classTable = 0x9000n;
const classAddress = 0x9100n;
const className = 0x9300n;
const classNamespace = 0x9320n;
const fields = 0x9400n;
const runtimeInfo = 0x9500n;
const vtables = 0x9600n;
const staticTable = 0x9700n;

const moduleImage = new Uint8Array(0x1000);
const moduleView = new DataView(moduleImage.buffer);
const writeModuleU16 = (offset, value) => moduleView.setUint16(offset, value, true);
const writeModuleU32 = (offset, value) => moduleView.setUint32(offset, value, true);

// A minimal PE32+ image exporting mono_assembly_foreach. Its first instruction
// references Mono's assembly-list global through a RIP-relative displacement.
writeModuleU16(0x00, 0x5a4d);
writeModuleU32(0x3c, 0x80);
writeModuleU32(0x80, 0x00004550);
writeModuleU16(0x98, 0x020b);
writeModuleU32(0x108, 0x200);
writeModuleU32(0x10c, 0x100);
writeModuleU32(0x214, 1);
writeModuleU32(0x218, 1);
writeModuleU32(0x21c, 0x300);
writeModuleU32(0x220, 0x310);
writeModuleU32(0x224, 0x320);
writeModuleU32(0x300, 0x500);
writeModuleU32(0x310, 0x400);
writeModuleU16(0x320, 0);
moduleImage.set(new TextEncoder().encode("mono_assembly_foreach\0"), 0x400);
moduleImage.set([0x48, 0x8b, 0x0d], 0x500);
writeModuleU32(0x503, Number(assemblyList - (moduleBase + 0x507n)));

const memory = new Map();
const writeBytes = (address, bytes) => {
    bytes.forEach((value, index) => memory.set(address + BigInt(index), value));
};
const writeNumber = (address, size, write) => {
    const buffer = new ArrayBuffer(size);
    write(new DataView(buffer));
    writeBytes(address, new Uint8Array(buffer));
};
const writeU32 = (address, value) => writeNumber(
    address,
    4,
    view => view.setUint32(0, value, true),
);
const writeI32 = (address, value) => writeNumber(
    address,
    4,
    view => view.setInt32(0, value, true),
);
const writeU64 = (address, value) => writeNumber(
    address,
    8,
    view => view.setBigUint64(0, value, true),
);
const writeF64 = (address, value) => writeNumber(
    address,
    8,
    view => view.setFloat64(0, value, true),
);
const writeUtf8 = (address, value, capacity = 256) => {
    const bytes = new Uint8Array(capacity);
    bytes.set(new TextEncoder().encode(value));
    writeBytes(address, bytes);
};

writeU64(assemblyList, link);
writeU64(link, assembly);
writeU64(link + 8n, 0n);
writeU64(assembly + 0x10n, assemblyName);
writeU64(assembly + 0x60n, imageAddress);
writeUtf8(assemblyName, "Assembly-CSharp");

const classCache = imageAddress + 0x4c0n;
writeI32(classCache + 0x18n, 1);
writeU64(classCache + 0x20n, classTable);
writeU64(classTable, classAddress);
writeU64(classAddress + 0x30n, 0n);
writeU64(classAddress + 0x48n, className);
writeU64(classAddress + 0x50n, classNamespace);
writeU32(classAddress + 0x5cn, 1);
writeU64(classAddress + 0x98n, fields);
writeU64(classAddress + 0xd0n, runtimeInfo);
writeI32(classAddress + 0x100n, 3);
writeU64(classAddress + 0x108n, 0n);
writeUtf8(className, "AutoSplitterData");
writeUtf8(classNamespace, "");

const defineField = (index, name, offset) => {
    const field = fields + BigInt(index) * 0x20n;
    const nameAddress = 0xa000n + BigInt(index) * 0x100n;
    writeU64(field + 0x8n, nameAddress);
    writeU32(field + 0x18n, offset);
    writeUtf8(nameAddress, name);
};
defineField(0, "inGameTime", 0x10);
defineField(1, "levelID", 0x18);
defineField(2, "isRunning", 0x20);

writeU64(runtimeInfo + 8n, vtables);
writeU64(vtables + 0x48n, staticTable);
writeF64(staticTable + 0x10n, 12.5);
writeI32(staticTable + 0x18n, 1);
writeBytes(staticTable + 0x20n, [0]);

const host = await SplitScriptHost.instantiate(wasmPath);
host.addProcess("ARTIFICIAL.exe", {
    modules: {
        "mono-2.0-bdwgc.dll": {
            address: moduleBase,
            size: BigInt(moduleImage.length),
        },
    },
    ranges: [{ address: moduleBase, bytes: moduleImage, flags: 5n }],
    read({ address, outputPointer, length, host: attachedHost }) {
        const moduleEnd = moduleBase + BigInt(moduleImage.length);
        if (address >= moduleBase && address + BigInt(length) <= moduleEnd) {
            const offset = Number(address - moduleBase);
            attachedHost.bytes(outputPointer, length).set(
                moduleImage.subarray(offset, offset + length),
            );
            return true;
        }

        const output = attachedHost.bytes(outputPointer, length);
        for (let index = 0; index < length; index += 1) {
            const value = memory.get(address + BigInt(index));
            if (value === undefined) return false;
            output[index] = value;
        }
        return true;
    },
});

host.start();
host.update(12);

writeBytes(staticTable + 0x20n, [1]);
host.updateUntil(
    () => host.timerCalls.starts === 1,
    "ARTIFICIAL did not start after isRunning became true",
);

writeF64(staticTable + 0x10n, 42.25);
writeI32(staticTable + 0x18n, 2);
host.update();
if (host.timerCalls.splits !== 1) {
    throw new Error(`level transition did not split: ${host.json(host.summary())}`);
}

writeBytes(staticTable + 0x20n, [0]);
host.updateUntil(
    () => host.timerCalls.resets === 1,
    "ARTIFICIAL did not reset after isRunning became false",
);

if (!host.timerCalls.gameTimes.some(([seconds, nanoseconds]) => (
    seconds === 42n && nanoseconds === 250_000_000
))) {
    throw new Error(`managed game time was not forwarded: ${host.json(host.summary())}`);
}
if (host.timerCalls.pauses === 0) {
    throw new Error(`isLoading did not pause game time: ${host.json(host.summary())}`);
}

console.log(host.json({
    starts: host.timerCalls.starts,
    splits: host.timerCalls.splits,
    resets: host.timerCalls.resets,
    gameTimes: host.timerCalls.gameTimes,
}));
