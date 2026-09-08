import assert from 'node:assert/strict';
import test from 'node:test';

import { GuestMemory } from '../src/debugger/asr/memory.ts';
import { neutralImport } from '../src/debugger/asr/neutralImports.ts';
import { DebuggerTimer } from '../src/debugger/asr/timer.ts';

test('guest memory writes ASR host strings and reports required capacity', () => {
    const wasmMemory = new WebAssembly.Memory({ initial: 1 });
    const memory = new GuestMemory();
    memory.bind(wasmMemory);
    const view = new DataView(wasmMemory.buffer);

    view.setUint32(32, 3, true);
    assert.equal(memory.writeHostString(64, 32, 'windows'), 0);
    assert.equal(view.getUint32(32, true), 7);

    view.setUint32(32, 7, true);
    assert.equal(memory.writeHostString(64, 32, 'windows'), 1);
    assert.equal(memory.readString(64, 7), 'windows');
});

test('guest memory rejects out-of-bounds and invalid UTF-8 reads', () => {
    const wasmMemory = new WebAssembly.Memory({ initial: 1 });
    const memory = new GuestMemory();
    memory.bind(wasmMemory);
    new Uint8Array(wasmMemory.buffer)[0] = 0xff;

    assert.throws(() => memory.readString(65_535, 2), WebAssembly.RuntimeError);
    assert.throws(() => memory.readString(0, 1), TypeError);
});

test('debugger timer mirrors ASR timer transitions and variables', () => {
    let changes = 0;
    const logs: string[] = [];
    const timer = new DebuggerTimer(
        () => changes += 1,
        message => logs.push(message.message),
    );

    assert.equal(timer.stateNumber(), 0);
    assert.equal(timer.currentSplitIndex(), -1n);
    timer.start();
    timer.split();
    timer.skipSplit();
    assert.equal(timer.stateNumber(), 1);
    assert.equal(timer.currentSplitIndex(), 2n);
    assert.equal(timer.segmentSplitted(0n), 1);
    assert.equal(timer.segmentSplitted(1n), 0);
    assert.equal(timer.segmentSplitted(2n), -1);

    timer.undoSplit();
    timer.setGameTime(12n, 500_000_000);
    timer.pauseGameTime();
    timer.setVariable('Route', 'Any%');
    assert.deepEqual(timer.snapshot(), {
        state: 'running',
        gameTimeSeconds: 12.5,
        gameTimeState: 'paused',
        splitIndex: 1,
        variables: { Route: 'Any%' },
    });
    assert(changes >= 7);
    assert.deepEqual(logs.slice(0, 4), [
        'Timer started.',
        'Splitted.',
        'Split skipped.',
        'Split undone.',
    ]);

    timer.reset();
    assert.equal(timer.stateNumber(), 0);
    assert.deepEqual(timer.snapshot().variables, {});
});

test('neutral ASR imports preserve WebAssembly i64 result types', () => {
    assert.equal(neutralImport('env.process_attach')(), 0n);
    assert.equal(neutralImport('env.settings_map_len')(), 0n);
    assert.equal(neutralImport('env.settings_list_len')(), 0n);
    assert.equal(neutralImport('env.settings_map_get')(), 0);
    assert.equal(neutralImport('env.settings_list_get')(), 0);
    assert.equal(neutralImport('env.setting_value_get_i64')(), 0);
});
