import { SplitScriptHost } from "./support/splitscript_host.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/host_metadata_runtime.mjs <host-metadata.wasm>");
}

const host = await SplitScriptHost.instantiate(wasmPath, {
    operatingSystem: "windows",
    architecture: "x86_64",
});
host.addProcess("game.exe", { path: "/mnt/c/Games/game.exe" });
host.start();
host.updateUntil(
    () => host.variables.has("Host Metadata"),
    "the script never published host metadata",
);

const observed = host.variables.get("Host Metadata");
const expected = "/mnt/c/Games/game.exe|windows|x86_64";
if (observed !== expected) {
    throw new Error(`unexpected host metadata: ${JSON.stringify({ expected, observed })}`);
}

console.log(JSON.stringify({ observed }));
