import { SplitScriptHost } from "./support/splitscript_host.mjs";
import { createMonoV2Fixture } from "./support/mono_v2_fixture.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/managed_instances_mono_runtime.mjs <autosplitter.wasm>");
}

const mono = createMonoV2Fixture({
    className: "Enemy",
    fields: [{ name: "health", offset: 0x10 }],
});
const heapBase = 0x10000n;
const heap = new Uint8Array(0x90000);
mono.writeBytes(heapBase, heap);
// Mono objects identify their class through the active domain vtable stored in
// the first object word, not through the metadata-class address itself.
mono.writeU64(heapBase + 0x1000n, 0x9600n);
mono.writeU64(heapBase + 0x81000n, 0x9600n);
mono.process.ranges.push({ address: heapBase, bytes: heap, flags: 0x6n });

const host = await SplitScriptHost.instantiate(wasmPath);
host.addProcess("game.exe", mono.process);
host.start();
let completedAt;
for (let tick = 0; tick < 80 && host.messages.length === 0; tick += 1) {
    host.update();
    if (host.messages.length !== 0) completedAt = tick + 1;
}

if (JSON.stringify(host.messages) !== JSON.stringify(["2"]) || completedAt < 3) {
    throw new Error(`unexpected Mono managed instances: ${host.json({
        messages: host.messages,
        completedAt,
        summary: host.summary(),
    })}`);
}

console.log(host.json({ messages: host.messages, completedAt }));
