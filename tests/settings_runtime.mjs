import { SplitScriptHost } from "./support/splitscript_host.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/settings_runtime.mjs <settings.wasm>");
}

const host = await SplitScriptHost.instantiate(wasmPath);
host.addProcess("explorer.exe");
host.start();

// The first complete state poll initializes old and current without running
// lifecycle code. The next poll observes the initialized settings snapshot.
host.update(2);

const expectVariable = (name, expected) => {
    const actual = host.variables.get(name);
    if (actual !== expected) {
        throw new Error(`unexpected variable ${JSON.stringify(name)}: ${JSON.stringify({
            expected,
            actual,
        })}`);
    }
};

expectVariable("Auto Splitting", "enabled");
expectVariable("Auto Splitting by Key", "enabled");
expectVariable("Previous Auto Splitting by Key", "enabled");
expectVariable("Unknown Setting by Key", "disabled");
expectVariable("Choice Setting by Key", "disabled");
expectVariable("Contains Boolean Key", "true");
expectVariable("Contains Choice Key", "true");
expectVariable("Contains File Key", "true");
expectVariable("Contains Heading Key", "false");
expectVariable("Contains Unknown Key", "false");
expectVariable("Capture Source", "Executable Name");
expectVariable("Layout File", "");

host.setSetting("auto-splitting", false);
host.setSetting("captureMode", "FullPath");
host.setSetting("layoutFile", "/mnt/c/layout.json");
host.setSetting("liveReload", false);
host.setSetting("verboseLogging", true);
host.update();

expectVariable("Auto Splitting", "disabled");
expectVariable("Auto Splitting by Key", "disabled");
expectVariable("Previous Auto Splitting by Key", "enabled");
expectVariable("Capture Source", "Full Path");
expectVariable("Layout File", "/mnt/c/layout.json");
if (!host.messages.includes("Live Reload is now disabled")) {
    throw new Error(`oldSettings did not rotate: ${JSON.stringify(host.messages)}`);
}
if (!host.messages.includes("Verbose settings diagnostics tick")) {
    throw new Error("live verbose setting was not observed");
}

const titles = host.widgets
    .filter(([kind]) => kind === "title")
    .map(([, , description, level]) => [description, level]);
const expectedTitles = [
    ["General", 0],
    ["Paths", 1],
    ["Advanced", 1],
    ["Diagnostics", 2],
];
if (JSON.stringify(titles) !== JSON.stringify(expectedTitles)) {
    throw new Error(`unexpected title hierarchy: ${JSON.stringify(titles)}`);
}
if (host.widgets.filter(([kind]) => kind === "option").length !== 3) {
    throw new Error(`unexpected choice options: ${JSON.stringify(host.widgets)}`);
}
if (host.filters.length !== 5) {
    throw new Error(`expected five file filters, got ${JSON.stringify(host.filters)}`);
}
if (host.tooltips.size !== 9) {
    throw new Error(`expected nine tooltips, got ${host.tooltips.size}`);
}
if (
    host.tooltips.get("auto-splitting")
    !== "Turns the example split logic on or off without unloading the auto splitter."
) {
    throw new Error("multiline documentation comments were not folded into a tooltip");
}
if (host.settingsMaps.size !== 0 || host.settingValues.size !== 0) {
    throw new Error("settings fixture handles leaked");
}

console.log(JSON.stringify({
    widgets: host.widgets,
    filters: host.filters,
    tooltips: Object.fromEntries(host.tooltips),
    variables: Object.fromEntries(host.variables),
    messages: host.messages,
}));
