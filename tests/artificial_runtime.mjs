import { SplitScriptHost } from "./support/splitscript_host.mjs";
import { createMonoV2Fixture } from "./support/mono_v2_fixture.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/artificial_runtime.mjs <artificial.wasm>");
}

const mono = createMonoV2Fixture({
    className: "AutoSplitterData",
    fields: [
        { name: "inGameTime", offset: 0x10 },
        { name: "levelID", offset: 0x18 },
        { name: "isRunning", offset: 0x20 },
    ],
});
mono.writeF64(mono.staticTable + mono.fieldOffsets.get("inGameTime"), 12.5);
mono.writeI32(mono.staticTable + mono.fieldOffsets.get("levelID"), 1);
mono.writeBytes(mono.staticTable + mono.fieldOffsets.get("isRunning"), [0]);

const host = await SplitScriptHost.instantiate(wasmPath);
host.addProcess("ARTIFICIAL.exe", mono.process);
host.start();
host.update(12);

mono.writeBytes(mono.staticTable + mono.fieldOffsets.get("isRunning"), [1]);
host.updateUntil(
    () => host.timerCalls.starts === 1,
    "ARTIFICIAL did not start after isRunning became true",
);

mono.writeF64(mono.staticTable + mono.fieldOffsets.get("inGameTime"), 42.25);
mono.writeI32(mono.staticTable + mono.fieldOffsets.get("levelID"), 2);
host.update();
if (host.timerCalls.splits !== 1) {
    throw new Error(`level transition did not split: ${host.json(host.summary())}`);
}

mono.writeBytes(mono.staticTable + mono.fieldOffsets.get("isRunning"), [0]);
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
