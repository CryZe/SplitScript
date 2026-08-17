import { SplitScriptHost } from "./support/splitscript_host.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/neon_white_runtime.mjs <neon-white.wasm>");
}

const moduleBase = 0x10000000n;
const pointers = new Map();
let nextPointer = 0x20000000n;

const pointer = (address) => {
    let value = pointers.get(address);
    if (value === undefined) {
        value = nextPointer;
        nextPointer += 0x10000n;
        pointers.set(address, value);
    }
    return value;
};

const path = (...offsets) => {
    let address = moduleBase + offsets[0];
    for (const offset of offsets.slice(1)) {
        address = pointer(address) + offset;
    }
    return address;
};

const addresses = {
    levelPlaythroughMicroseconds: path(0x199cdc0n, 0x18n, 0x40n, 0x28n, 0x48n, 0x20n),
    levelRushMicroseconds: path(
        0x1930010n,
        0x10n,
        0xd0n,
        0x8n,
        0x60n,
        0x50n,
        0x0n,
        0x1c0n,
        0x10n,
        0x20n,
    ),
    levelId: path(0x199cdc0n, 0x18n, 0x40n, 0x28n, 0x30n, 0x20n, 0x14n),
    levelScene: path(0x1a058e0n, 0x48n, 0x10n, 0x18n),
};

const state = {
    levelPlaythroughMicroseconds: 0n,
    levelRushMicroseconds: 0n,
    levelId: "TUT_MOVEMENT",
    levelScene: "menu.unity",
};
const failedFields = new Set();
const reads = new Map(Object.keys(addresses).map(name => [name, 0]));

const host = await SplitScriptHost.instantiate(wasmPath);
host.addProcess("Neon White.exe", {
    modules: {
        "UnityPlayer.dll": { address: moduleBase, size: 0x3000000n },
    },
    read({ address, outputPointer, length, host: attachedHost }) {
        const pointerValue = pointers.get(address);
        if (pointerValue !== undefined && length === 8) {
            attachedHost.view().setBigUint64(outputPointer, pointerValue, true);
            return true;
        }

        const field = Object.entries(addresses).find(([, value]) => value === address)?.[0];
        if (field === undefined) {
            throw new Error(`unexpected process read ${address.toString(16)} (${length})`);
        }
        reads.set(field, reads.get(field) + 1);
        if (failedFields.has(field)) return false;

        if (field === "levelPlaythroughMicroseconds" || field === "levelRushMicroseconds") {
            if (length !== 8) throw new Error(`unexpected ${field} width ${length}`);
            attachedHost.view().setBigInt64(outputPointer, state[field], true);
            return true;
        }

        if (length !== 255) throw new Error(`unexpected ${field} width ${length}`);
        const output = attachedHost.bytes(outputPointer, length);
        output.fill(0);
        output.set(attachedHost.encoder.encode(state[field]));
        return true;
    },
});

const actionCounts = () => ({
    starts: host.timerCalls.starts,
    splits: host.timerCalls.splits,
    resets: host.timerCalls.resets,
});
const assertActions = (expected, context) => {
    const actual = actionCounts();
    if (Object.entries(expected).some(([key, value]) => actual[key] !== value)) {
        throw new Error(`${context}: ${host.json({ expected, actual, summary: host.summary() })}`);
    }
};
const assertGameTime = (milliseconds, context) => {
    const actual = host.timerCalls.gameTimes.at(-1);
    const expected = [
        BigInt(Math.trunc(milliseconds / 1000)),
        (milliseconds % 1000) * 1_000_000,
    ];
    if (actual?.[0] !== expected[0] || actual?.[1] !== expected[1]) {
        throw new Error(`${context}: ${host.json({ expected, actual })}`);
    }
};

host.start();

// The first complete read seeds equal old/current snapshots and emits no timer
// action, even though the initial scene differs from every zero/default value.
host.update();
assertActions({ starts: 0, splits: 0, resets: 0 }, "initial snapshot emitted an action");
if (host.timerCalls.gameTimes.length !== 0) {
    throw new Error("initial snapshot unexpectedly set game time");
}

// Entering the first Level Rush scene starts a stopped timer.
state.levelScene = "id/TUT_MOVEMENT.unity";
host.update();
assertActions({ starts: 1, splits: 0, resets: 0 }, "first scene did not start the timer");

// A genuine level transition splits, while an empty one-frame ID is replaced
// with the preceding accepted ID and cannot split.
state.levelId = "LEVEL_1";
state.levelScene = "id/LEVEL_1.unity";
state.levelPlaythroughMicroseconds = 5_000_000n;
host.update();
assertActions({ starts: 1, splits: 1, resets: 0 }, "level transition did not split");
assertGameTime(5_000, "current playthrough time was not reported");

state.levelId = "";
state.levelPlaythroughMicroseconds = 6_000_000n;
host.update();
assertActions({ starts: 1, splits: 1, resets: 0 }, "empty level ID caused a split");
assertGameTime(6_000, "empty level ID disturbed game time");

// Establish an accumulated rush time, then ensure its transient loading zero
// is suppressed outside a first-level scene.
state.levelId = "LEVEL_1";
state.levelRushMicroseconds = 10_000_000n;
state.levelPlaythroughMicroseconds = 0n;
host.update();
assertGameTime(10_000, "accumulated rush time was not reported");

state.levelRushMicroseconds = 0n;
state.levelPlaythroughMicroseconds = 2_000_000n;
host.update();
assertGameTime(12_000, "transient zero rush time was not suppressed");

// When the accumulated timer advances before the playthrough timer resets,
// exclude the current playthrough. Re-enable it when the next playthrough is
// created, then observe the following positive value normally.
state.levelRushMicroseconds = 12_000_000n;
host.update();
assertGameTime(12_000, "rush rollover double-counted the current level");

state.levelPlaythroughMicroseconds = -1n;
host.update();
assertGameTime(11_999, "new playthrough did not restore current-level timing");

state.levelPlaythroughMicroseconds = 1_000_000n;
host.update();
assertGameTime(13_000, "restored playthrough time was not included");

// A required field read failure retains that field while successful siblings
// still advance. Returning to the rush menu therefore splits once for the
// scene, and restoring the changed level ID splits on the following tick.
failedFields.add("levelId");
state.levelId = "LEVEL_2";
state.levelScene = "nu.unity";
host.update();
assertActions({ starts: 1, splits: 2, resets: 0 }, "failed field blocked a successful sibling");

failedFields.delete("levelId");
host.update();
assertActions({ starts: 1, splits: 3, resets: 0 }, "retained level ID did not later advance");

// The same first-scene predicate resets a running timer and starts a stopped
// timer on a later transition.
state.levelId = "TUT_MOVEMENT";
state.levelScene = "id/TUT_MOVEMENT.unity";
host.update();
assertActions({ starts: 1, splits: 3, resets: 1 }, "first scene did not reset a running timer");

state.levelScene = "menu.unity";
host.update();
state.levelScene = "id/TUT_MOVEMENT.unity";
host.update();
assertActions({ starts: 2, splits: 3, resets: 1 }, "first scene did not restart a stopped timer");

// Closing and reopening the process yields a fresh equal snapshot. Even with
// entirely different values, reattachment itself must not split or reset.
host.setProcessOpen("Neon White.exe", false);
host.updateUntil(() => host.detaches.length === 1, "Neon White did not detach");
host.setProcessOpen("Neon White.exe", true);
state.levelId = "LEVEL_AFTER_REATTACH";
state.levelScene = "somewhere-else.unity";
state.levelRushMicroseconds = 20_000_000n;
state.levelPlaythroughMicroseconds = 0n;
const gameTimesBeforeReattach = host.timerCalls.gameTimes.length;
host.update();
assertActions({ starts: 2, splits: 3, resets: 1 }, "reattachment emitted an action");
if (host.timerCalls.gameTimes.length !== gameTimesBeforeReattach) {
    throw new Error("reattachment seed unexpectedly set game time");
}

host.update();
assertGameTime(20_000, "reattached snapshot did not drive later actions");

if (host.attachAttempts.some(name => name !== "Neon White.exe")) {
    throw new Error(`unexpected attachment candidate: ${host.json(host.attachAttempts)}`);
}
if ([...reads.values()].some(count => count < 2)) {
    throw new Error(`state fixture did not exercise every field: ${host.json(Object.fromEntries(reads))}`);
}

console.log(host.json({
    ...actionCounts(),
    detaches: host.detaches.length,
    gameTimes: host.timerCalls.gameTimes.length,
    reads: Object.fromEntries(reads),
}));
