import assert from 'node:assert/strict';
import test from 'node:test';

import { GuestMemory } from '../src/debugger/asr/memory.ts';
import { neutralImport } from '../src/debugger/asr/neutralImports.ts';
import { DebuggerTimer } from '../src/debugger/asr/timer.ts';
import { SettingsHost } from '../src/debugger/asr/settings.ts';
import { nativePathToWasi, WasiHost } from '../src/debugger/asr/wasi.ts';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

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
    assert.equal(neutralImport('env.settings_map_get')(), 0n);
    assert.equal(neutralImport('env.settings_list_get')(), 0n);
    assert.equal(neutralImport('env.setting_value_get_i64')(), 0);
});

test('settings host registers widgets and refreshes values through 64-bit handles', () => {
    const wasmMemory = new WebAssembly.Memory({ initial: 1 });
    const memory = new GuestMemory();
    memory.bind(wasmMemory);
    const host = new SettingsHost(memory, [{
        key: 'enabled',
        value: { type: 'bool', value: false },
    }]);
    const imports = host.imports() as Record<string, (...arguments_: unknown[]) => unknown>;
    const strings = writeStrings(memory, 256, ['enabled', 'Enabled', 'mode', 'Mode', 'fast', 'Fast']);

    assert.equal(imports.user_settings_add_bool(
        strings.enabled.pointer,
        strings.enabled.length,
        strings.Enabled.pointer,
        strings.Enabled.length,
        1,
    ), 0);
    imports.user_settings_add_choice(
        strings.mode.pointer,
        strings.mode.length,
        strings.Mode.pointer,
        strings.Mode.length,
        strings.fast.pointer,
        strings.fast.length,
    );
    assert.equal(imports.user_settings_add_choice_option(
        strings.mode.pointer,
        strings.mode.length,
        strings.fast.pointer,
        strings.fast.length,
        strings.Fast.pointer,
        strings.Fast.length,
    ), 1);

    const map = imports.settings_map_load() as bigint;
    assert.equal(typeof map, 'bigint');
    const value = imports.settings_map_get(
        map,
        strings.enabled.pointer,
        strings.enabled.length,
    ) as bigint;
    assert.equal(typeof value, 'bigint');
    assert.equal(imports.setting_value_get_bool(value, 32), 1);
    assert.equal(new Uint8Array(wasmMemory.buffer)[32], 0);

    host.set('enabled', true);
    assert.equal(host.snapshot().map[0].value.type, 'bool');
    assert.equal(host.snapshot().widgets.length, 2);
});

test('read-only WASI host opens and reads files through the /mnt preopen', () => {
    const directory = mkdtempSync(join(tmpdir(), 'splitscript-wasi-'));
    const file = join(directory, 'probe.txt');
    writeFileSync(file, 'wasi probe');
    try {
        const wasmMemory = new WebAssembly.Memory({ initial: 1 });
        const memory = new GuestMemory();
        memory.bind(wasmMemory);
        const host = new WasiHost(memory, file, () => {});
        const imports = host.imports() as Record<string, (...arguments_: unknown[]) => unknown>;
        const relative = nativePathToWasi(file).slice('/mnt/'.length);
        memory.writeBytes(128, new TextEncoder().encode(relative));
        assert.equal(imports.path_open(3, 0, 128, relative.length, 0, 2n, 0n, 0, 32), 0);
        const descriptor = memory.readU32(32);
        memory.writeU32(48, 256);
        memory.writeU32(52, 32);
        assert.equal(imports.fd_read(descriptor, 48, 1, 40), 0);
        assert.equal(memory.readString(256, memory.readU32(40)), 'wasi probe');
        assert.equal(imports.fd_close(descriptor), 0);

        assert.equal(imports.environ_sizes_get(60, 64), 0);
        assert.equal(memory.readU32(60), 1);
        host.dispose();
    } finally {
        rmSync(directory, { recursive: true, force: true });
    }
});

function writeStrings(
    memory: GuestMemory,
    start: number,
    values: readonly string[],
): Record<string, { pointer: number; length: number }> {
    const result: Record<string, { pointer: number; length: number }> = {};
    let pointer = start;
    for (const value of values) {
        const bytes = new TextEncoder().encode(value);
        memory.writeBytes(pointer, bytes);
        result[value] = { pointer, length: bytes.length };
        pointer += bytes.length;
    }
    return result;
}
