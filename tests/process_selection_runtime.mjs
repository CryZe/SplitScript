import assert from "node:assert/strict";
import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/process_selection_runtime.mjs <selection.wasm>");
}

const bytes = fs.readFileSync(wasmPath);
const candidates = [10n, 20n, 30n];
const listCalls = [];
const attachAttempts = [];
const detaches = [];
const tickRates = [];
let starts = 0;
let instance;

const env = {
    timer_get_state: () => 0,
    timer_start: () => { starts += 1; },
    runtime_set_tick_rate: (rate) => { tickRates.push(rate); },
    process_list_by_name(namePointer, nameLength, listPointer, lengthPointer) {
        const memory = instance.exports.memory.buffer;
        const name = new TextDecoder().decode(
            new Uint8Array(memory, namePointer, nameLength),
        );
        assert.equal(name, "game.exe");
        const view = new DataView(memory);
        const capacity = view.getUint32(lengthPointer, true);
        listCalls.push(capacity);
        for (let index = 0; index < Math.min(capacity, candidates.length); index += 1) {
            view.setBigUint64(listPointer + index * 8, candidates[index], true);
        }
        view.setUint32(lengthPointer, candidates.length, true);
        return 1;
    },
    process_attach_by_pid(pid) {
        attachAttempts.push(pid);
        return pid;
    },
    process_detach(handle) { detaches.push(handle); },
    process_is_open: () => 1,
    process_read(handle, address, outputPointer, length) {
        assert.equal(length, 1);
        if (address === 0x2000n) {
            if (handle === 20n) return 0;
            new Uint8Array(instance.exports.memory.buffer, outputPointer, 1)[0] =
                handle === 30n ? 42 : 0;
            return 1;
        }
        assert.equal(handle, 30n);
        assert.equal(address, 0x1000n);
        new Uint8Array(instance.exports.memory.buffer, outputPointer, 1)[0] = 7;
        return 1;
    },
};

({ instance } = await WebAssembly.instantiate(bytes, { env }));
assert.equal("process_attach" in env, false);
instance.exports._start();
instance.exports.update();
instance.exports.update();

assert.deepEqual(listCalls, [0, 3]);
assert.deepEqual(attachAttempts, candidates);
assert.deepEqual(detaches, [10n, 20n]);
assert.deepEqual(tickRates, [1, 120]);
assert.equal(starts, 1);

console.log(JSON.stringify({
    listCalls,
    attachAttempts: attachAttempts.map(String),
    detaches: detaches.map(String),
    starts,
}));
