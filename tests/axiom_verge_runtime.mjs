import { SplitScriptHost } from "./support/splitscript_host.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/axiom_verge_runtime.mjs <axiom-verge.wasm>");
}

const signatureBase = 0x1000n;
const signatureOffset = 0x20;
const signatureBytes = new Uint8Array(0x200);
signatureBytes[signatureOffset + 4] = 0x04;
signatureBytes[signatureOffset + 20] = 0xf8;
signatureBytes[signatureOffset + 21] = 0x07;

const gameRoot = signatureBase + BigInt(signatureOffset + 4) + 0x144n;
const game = 0x2000n;
const checkpointList = 0x3000n;
const checkpointEntries = 0x3100n;
const itemList = 0x4000n;
const itemEntries = 0x4100n;
const keyPointList = 0x5000n;
const keyPointEntries = 0x5100n;
const memory = new Map();

const writeBytes = (address, values) => {
    values.forEach((value, index) => memory.set(address + BigInt(index), value));
};
const writeNumber = (address, size, write) => {
    const buffer = new ArrayBuffer(size);
    write(new DataView(buffer));
    writeBytes(address, new Uint8Array(buffer));
};
const writeI32 = (address, value) => writeNumber(
    address,
    4,
    view => view.setInt32(0, value, true),
);
const writeU32 = (address, value) => writeNumber(
    address,
    4,
    view => view.setUint32(0, Number(value), true),
);
const writeF64 = (address, value) => writeNumber(
    address,
    8,
    view => view.setFloat64(0, value, true),
);
const writeUtf16 = (address, value) => {
    const bytes = [];
    for (const character of value) {
        const codeUnit = character.charCodeAt(0);
        bytes.push(codeUnit & 0xff, codeUnit >> 8);
    }
    writeBytes(address, bytes);
};

writeU32(gameRoot + 0xe0n, game);
writeI32(game + 0xb4n, 100);
writeF64(game + 0xcn, 2.0);
writeU32(game + 0x48n, checkpointList);
writeU32(game + 0x30n, itemList);
writeU32(game + 0x34n, keyPointList);
writeI32(checkpointList + 0xcn, 0);
writeI32(itemList + 0xcn, 0);
writeI32(keyPointList + 0xcn, 0);
writeU32(checkpointList + 0x4n, checkpointEntries);
writeU32(itemList + 0x4n, itemEntries);
writeU32(keyPointList + 0x4n, keyPointEntries);

const defineCheckpoint = (index, value) => {
    const entry = 0x3200n + BigInt(index) * 0x100n;
    const text = entry + 0x40n;
    writeU32(checkpointEntries + 0x8n + BigInt(index * 4), entry);
    writeU32(entry + 0xcn, text);
    writeI32(text + 0x4n, value.length);
    writeUtf16(text + 0x8n, value);
};
const defineItem = (index, value) => {
    const entry = 0x4200n + BigInt(index) * 0x100n;
    const text = entry + 0x40n;
    writeU32(itemEntries + 0x8n + BigInt(index * 4), entry);
    writeU32(entry + 0x4n, text);
    writeI32(text + 0x4n, value.length);
    writeUtf16(text + 0x8n, value);
};
const defineKeyPoint = (index, value) => {
    const entry = 0x5200n + BigInt(index) * 0x100n;
    writeU32(keyPointEntries + 0x8n + BigInt(index * 4), entry);
    writeI32(entry + 0x4n, value.length);
    writeUtf16(entry + 0x8n, value);
};

defineCheckpoint(0, "Gir-Tab");
defineCheckpoint(1, "Xedur");
defineCheckpoint(2, "Telal");
defineItem(0, "DataDisruptor");
defineKeyPoint(0, "FirstDeath");

let scanReads = 0;
const host = await SplitScriptHost.instantiate(wasmPath, {
    settings: { reset_death: true },
});
host.addProcess("AxiomVerge.exe", {
    modules: {
        "steam_api.dll": {
            address: 0x7000n,
            size: 0x1000n,
        },
    },
    ranges: [{ address: signatureBase, bytes: signatureBytes, flags: 2n }],
    read({ address, outputPointer, length, host: attachedHost }) {
        const rangeEnd = signatureBase + BigInt(signatureBytes.length);
        if (address >= signatureBase && address + BigInt(length) <= rangeEnd) {
            scanReads += 1;
            const offset = Number(address - signatureBase);
            attachedHost.bytes(outputPointer, length).set(
                signatureBytes.subarray(offset, offset + length),
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
host.updateUntil(
    () => host.timerCalls.starts === 1,
    "Axiom Verge did not attach, discover its layout, and start",
);
host.update();

if (scanReads !== 1) {
    throw new Error(`expected one bounded process-wide scan read, got ${scanReads}`);
}
const booleanKeys = new Set(
    host.widgets.filter(([kind]) => kind === "bool").map(([, key]) => key),
);
if (booleanKeys.size !== 119
    || !booleanKeys.has("DIALOGUE_SecurityWormMeet")
    || !booleanKeys.has("Note28")
    || !booleanKeys.has("bosses")) {
    throw new Error(`unexpected Axiom Verge settings registration: ${host.json(host.widgets)}`);
}

writeF64(game + 0xcn, 120.0);
writeI32(checkpointList + 0xcn, 1);
host.update();
if (host.timerCalls.splits !== 0) {
    throw new Error("disabled Gir-Tab checkpoint unexpectedly split");
}

writeI32(checkpointList + 0xcn, 2);
host.setSetting("bosses", false);
host.update();
if (host.timerCalls.splits !== 0) {
    throw new Error("disabled boss parent did not gate the enabled Xedur checkpoint");
}

host.setSetting("bosses", true);
writeI32(checkpointList + 0xcn, 3);
host.update();
if (host.timerCalls.splits !== 1) {
    throw new Error("disabled events did not advance to the enabled Telal checkpoint");
}

writeI32(itemList + 0xcn, 1);
host.update();
if (host.timerCalls.splits !== 2) {
    throw new Error("enabled Data Disruptor item did not split");
}

writeI32(keyPointList + 0xcn, 1);
host.updateUntil(
    () => host.timerCalls.resets === 1,
    "FirstDeath did not request a reset",
    3,
);

if (!host.timerCalls.gameTimes.some(([seconds, nanoseconds]) => (
    seconds === 2n && nanoseconds === 0
))) {
    throw new Error(`sixty-hertz tick conversion was incorrect: ${host.json(host.timerCalls)}`);
}
if (!host.messages.includes("Checkpoint: Gir-Tab")
    || !host.messages.includes("Checkpoint: Xedur")
    || !host.messages.includes("Checkpoint: Telal")
    || !host.messages.includes("Item: DataDisruptor")
    || !host.messages.includes("Key point: FirstDeath")) {
    throw new Error(`event diagnostics were incomplete: ${host.json(host.messages)}`);
}

console.log(host.json({
    settings: booleanKeys.size,
    scanReads,
    starts: host.timerCalls.starts,
    splits: host.timerCalls.splits,
    resets: host.timerCalls.resets,
}));
