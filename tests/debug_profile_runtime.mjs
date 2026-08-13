import fs from "node:fs";

const [debugPath, releasePath] = process.argv.slice(2);
if (!debugPath || !releasePath) {
    throw new Error("usage: node tests/debug_profile_runtime.mjs <debug.wasm> <release.wasm>");
}

async function run(path) {
    const decoder = new TextDecoder();
    const messages = [];
    const values = [];
    let instance;
    const text = (pointer, length) => decoder.decode(
        new Uint8Array(instance.exports.memory.buffer, pointer, length),
    );
    const env = {
        timer_get_state: () => 0,
        runtime_set_tick_rate() {},
        process_attach: () => 1n,
        process_detach() {},
        process_is_open: () => 1,
        runtime_print_message(pointer, length) {
            messages.push(text(pointer, length));
        },
        timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
            if (text(keyPointer, keyLength) === "Profile Value") {
                values.push(text(valuePointer, valueLength));
            }
        },
    };
    ({ instance } = await WebAssembly.instantiate(fs.readFileSync(path), { env }));
    instance.exports._start();
    for (let tick = 0; tick < 3; tick += 1) instance.exports.update();
    return { messages, values };
}

const debug = await run(debugPath);
const release = await run(releasePath);
if (debug.values.at(-1) !== "1" || debug.messages.length === 0) {
    throw new Error(`debug profile did not retain debug statements: ${JSON.stringify(debug)}`);
}
if (release.values.at(-1) !== "0" || release.messages.length !== 0) {
    throw new Error(`release profile did not erase debug statements: ${JSON.stringify(release)}`);
}

console.log(JSON.stringify({ debug, release }));
