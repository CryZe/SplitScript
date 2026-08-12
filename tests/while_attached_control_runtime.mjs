import { SplitScriptHost } from "./support/splitscript_host.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/while_attached_control_runtime.mjs <control.wasm>");
}

const host = await SplitScriptHost.instantiate(wasmPath);
host.addProcess("game.exe");
host.start();

// Attach and seed the first snapshot. No attached action runs on this update.
host.updateUntil(() => host.attachAttempts.length === 1, "the script did not attach");

// Explicit false suppresses start even though the refreshed snapshot remains committed.
host.update();
if (host.messages.join() !== "1" || host.timerCalls.starts !== 0) {
    throw new Error(`false did not suppress start: ${host.json(host.summary())}`);
}

// Fallthrough defaults to true and allows start to run.
host.update();
if (host.messages.join() !== "1,2" || host.timerCalls.starts !== 1) {
    throw new Error(`fallthrough did not continue to start: ${host.json(host.summary())}`);
}

// Explicit false also suppresses every running-timer action.
host.update();
if (
    host.messages.join() !== "1,2,3"
    || host.timerCalls.pauses !== 0
    || host.timerCalls.gameTimes.length !== 0
    || host.timerCalls.resets !== 0
    || host.timerCalls.splits !== 0
) {
    throw new Error(`false did not suppress running actions: ${host.json(host.summary())}`);
}

// The next fallthrough evaluates the normal running-timer sequence.
host.update();
if (
    host.messages.join() !== "1,2,3,4"
    || host.timerCalls.pauses !== 1
    || host.timerCalls.gameTimes.length !== 1
    || host.timerCalls.resets !== 1
    || host.timerCalls.splits !== 0
) {
    throw new Error(`continued update did not evaluate timer actions: ${host.json(host.summary())}`);
}

console.log(host.json({
    messages: host.messages,
    timerCalls: host.timerCalls,
}));
