import { SplitScriptHost } from "./support/splitscript_host.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/process_exit_lifecycle_runtime.mjs <lifecycle.wasm>");
}

const host = await SplitScriptHost.instantiate(wasmPath);
const gameProcess = host.addProcess("game.exe");
host.start();

host.updateUntil(() => host.messages.includes("attach"), "the process did not attach");
if (host.messages.join() !== "detached,attach" || host.timerCalls.pauses !== 0) {
    throw new Error(`process exit ran during initial detachment: ${host.json(host.summary())}`);
}

host.setProcessOpen("game.exe", false);
host.update();
if (host.messages.join() !== "detached,attach,exit,detached" || host.timerCalls.pauses !== 1) {
    throw new Error(`first exit ordering was incorrect: ${host.json(host.summary())}`);
}

host.update(2);
if (host.messages.join() !== "detached,attach,exit,detached" || host.timerCalls.pauses !== 1) {
    throw new Error(`exit repeated while detached: ${host.json(host.summary())}`);
}

gameProcess.modules.set("ready.dll", {
    address: 0x1000n,
    size: 0x2000n,
    path: "ready.dll",
});
host.setProcessOpen("game.exe", true);
host.updateUntil(
    () => host.messages.filter((message) => message === "attach").length === 2,
    "the process did not reattach",
);
host.update();
host.setProcessOpen("game.exe", false);
host.update();
if (
    host.messages.join() !== "detached,attach,exit,detached,attach,exit,detached"
    || host.timerCalls.pauses !== 2
) {
    throw new Error(`second exit was not exactly once: ${host.json(host.summary())}`);
}

console.log(JSON.stringify({
    messages: host.messages,
    exits: host.timerCalls.pauses,
}));
