import { SplitScriptHost } from "./support/splitscript_host.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/action_defaults_runtime.mjs <actions.wasm>");
}

const host = await SplitScriptHost.instantiate(wasmPath, {
    settings: { enabled: true },
});
host.addProcess("game.exe");
host.start();

if (host.messages.join() !== "true" || host.tickRates.join() !== "1,42") {
    throw new Error(`setup did not run exactly once: ${host.json(host.summary())}`);
}

// A NotRunning tick evaluates start; its fallthrough must be false.
host.update();

// A Running tick evaluates all other timer actions. Their fallthroughs must
// leave pause/game-time state untouched and must not reset or split.
host.timerState = 1;
host.update(2);

const calls = host.timerCalls;
if (
    calls.starts
    || calls.splits
    || calls.resets
    || calls.pauses
    || calls.resumes
    || calls.gameTimes.length
) {
    throw new Error(`action fallthrough caused host calls: ${host.json(host.summary())}`);
}
if (host.messages.length !== 1 || host.tickRates.join() !== "1,42,120") {
    throw new Error(`setup ran again during update: ${host.json(host.summary())}`);
}

console.log(JSON.stringify({
    start: calls.starts,
    split: calls.splits,
    reset: calls.resets,
    pause: calls.pauses,
    resume: calls.resumes,
    gameTime: calls.gameTimes.length,
    print: host.messages.length,
    tickRate: host.tickRates.length,
}));
