import { SplitScriptHost } from "./support/splitscript_host.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/on_state_ready_runtime.mjs <on-state-ready.wasm>");
}

const host = await SplitScriptHost.instantiate(wasmPath);
const gameProcess = host.addProcess("game.exe", {
    ranges: [{ address: 0x100n, bytes: new Uint8Array(4) }],
});
const level = new DataView(gameProcess.ranges[0].bytes.buffer);
level.setUint32(0, 7, true);
host.start();

host.updateUntil(
    () => host.messages.length === 1,
    "the initial snapshot hook did not run",
);
if (host.messages[0] !== "ready 1: 7 -> 7") {
    throw new Error(`unexpected initial snapshot: ${host.json(host.summary())}`);
}

level.setUint32(0, 9, true);
host.update();
if (host.messages[1] !== "tick: 7 -> 9") {
    throw new Error(`the first ordinary update was not distinct: ${host.json(host.summary())}`);
}

host.setProcessOpen("game.exe", false);
host.updateUntil(() => host.detaches.length === 1, "the process did not detach");
host.setProcessOpen("game.exe", true);
level.setUint32(0, 11, true);
host.updateUntil(
    () => host.messages.includes("ready 2: 11 -> 11"),
    "the snapshot hook did not run after reattachment",
);

console.log(JSON.stringify({
    messages: host.messages,
    detaches: host.detaches.length,
}));
