import { SplitScriptHost } from "./support/splitscript_host.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/tick_rate_lifecycle_runtime.mjs <tick-rate.wasm>");
}

const host = await SplitScriptHost.instantiate(wasmPath);
host.addProcess("hundred.exe");
host.addProcess("hundred-twenty.exe", { open: false });
host.start();

host.updateUntil(
    () => host.tickRates.includes(100),
    "the first process never selected 100 Hz",
);

host.setProcessOpen("hundred.exe", false);
host.updateUntil(
    () => host.tickRates.length >= 3 && host.tickRates.at(-1) === 60,
    "detaching did not restore the authored 60 Hz baseline",
);

host.setProcessOpen("hundred-twenty.exe", true);
host.updateUntil(
    () => host.tickRates.includes(120),
    "the second process never selected 120 Hz",
);

host.setProcessOpen("hundred-twenty.exe", false);
host.updateUntil(
    () => host.tickRates.length >= 5 && host.tickRates.at(-1) === 60,
    "the second detach did not restore the authored baseline",
);

const expectedRates = [60, 100, 60, 120, 60];
if (JSON.stringify(host.tickRates) !== JSON.stringify(expectedRates)) {
    throw new Error(
        `unexpected tick-rate transitions: ${JSON.stringify({
            expectedRates,
            tickRates: host.tickRates,
        })}`,
    );
}
if (host.detaches.length !== 2) {
    throw new Error(`unexpected detach count: ${host.detaches.length}`);
}

console.log(JSON.stringify({
    tickRates: host.tickRates,
    attachAttempts: host.attachAttempts,
    detaches: host.detaches.length,
}));
