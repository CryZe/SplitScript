import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { Worker } from 'node:worker_threads';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(extension, '..', '..');
const temporary = await mkdtemp(join(tmpdir(), 'splitscript-debug-runtime-'));
const worker = new Worker(resolve(extension, 'dist', 'runtimeWorker.js'));
const wasiWorker = new Worker(resolve(extension, 'dist', 'runtimeWorker.js'));

try {
    const generatedWasm = resolve(temporary, 'runtime-probe.wasm');
    const compiled = spawnSync('cargo', [
        'run',
        '--quiet',
        '--bin',
        'splitc',
        '--',
        resolve(extension, 'test', 'fixtures', 'runtime-probe.split'),
        '--output',
        generatedWasm,
        '--profile',
        'debug',
    ], { cwd: repository, stdio: 'inherit', shell: false });
    if (compiled.error) {
        throw compiled.error;
    }
    assert.equal(compiled.status, 0);

    const ready = waitFor(worker, message => message.type === 'ready');
    const configured = waitFor(
        worker,
        message => message.type === 'snapshot'
            && message.snapshot.tickCount > 0
            && message.snapshot.tickRateHz === 30,
    );
    const setupLog = waitFor(
        worker,
        message => message.type === 'log'
            && message.source === 'autoSplitter'
            && message.message === 'SplitScript runtime worker ready',
    );
    const bytes = await readFile(generatedWasm);
    worker.postMessage({
        type: 'launch',
        wasm: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength),
        program: generatedWasm,
    });
    const readyMessage = await ready;
    assert(readyMessage.unsupportedImports.includes('env.process_attach'));
    assert(!readyMessage.unsupportedImports.some(name => name.includes('settings')));
    await setupLog;
    const running = await configured;
    assert(running.snapshot.memoryBytes > 0);
    assert.equal(running.snapshot.settings.widgets.length, 5);

    const changedSetting = waitFor(
        worker,
        message => message.type === 'snapshot'
            && message.snapshot.settings.map.some(entry => entry.key === 'label'
                && entry.value.type === 'string'
                && entry.value.value === 'changed'),
    );
    worker.postMessage({ type: 'setSetting', key: 'label', value: 'changed' });
    await changedSetting;

    const started = waitFor(
        worker,
        message => message.type === 'snapshot' && message.snapshot.timer.state === 'running',
    );
    worker.postMessage({ type: 'timerCommand', command: 'start' });
    await started;
    const reset = waitFor(
        worker,
        message => message.type === 'snapshot' && message.snapshot.timer.state === 'notRunning',
    );
    worker.postMessage({ type: 'timerCommand', command: 'reset' });
    await reset;

    const wasiInput = resolve(temporary, 'wasi-input.txt');
    const wasiSource = resolve(temporary, 'wasi-probe.split');
    const wasiWasm = resolve(temporary, 'wasi-probe.wasm');
    await writeFile(wasiInput, 'read through WASI', 'utf8');
    await writeFile(wasiSource, `
state "missing-wasi-probe.exe" {}
setup {
    let content = File.readAllText("${nativeToWasi(wasiInput)}") else "failed"
    setVariable("WASI", content)
}
`, 'utf8');
    const wasiCompiled = spawnSync('cargo', [
        'run', '--quiet', '--bin', 'splitc', '--', wasiSource,
        '--output', wasiWasm, '--profile', 'debug',
    ], { cwd: repository, stdio: 'inherit', shell: false });
    if (wasiCompiled.error) throw wasiCompiled.error;
    assert.equal(wasiCompiled.status, 0);
    const wasiReady = waitFor(wasiWorker, message => message.type === 'ready');
    const wasiRead = waitFor(
        wasiWorker,
        message => message.type === 'snapshot'
            && message.snapshot.timer.variables.WASI === 'read through WASI',
    );
    const wasiBytes = await readFile(wasiWasm);
    wasiWorker.postMessage({
        type: 'launch',
        wasm: wasiBytes.buffer.slice(wasiBytes.byteOffset, wasiBytes.byteOffset + wasiBytes.byteLength),
        program: wasiWasm,
        scriptPath: wasiSource,
    });
    const wasiReadyMessage = await wasiReady;
    assert(!wasiReadyMessage.unsupportedImports.some(name => name.startsWith('wasi_snapshot_preview1.')));
    await wasiRead;

    console.log('Production debug runtime probe passed: launch, tick, log, settings, WASI, timer controls.');
} finally {
    await worker.terminate();
    await wasiWorker.terminate();
    await rm(temporary, { recursive: true, force: true });
}

function nativeToWasi(file) {
    const normalized = file.replaceAll('\\', '/');
    const match = /^([a-zA-Z]):\/(.*)$/.exec(normalized);
    return match === null ? `/mnt${normalized}` : `/mnt/${match[1].toLowerCase()}/${match[2]}`;
}

function waitFor(worker, predicate, timeoutMs = 5_000) {
    return new Promise((resolvePromise, reject) => {
        const timeout = setTimeout(() => {
            cleanup();
            reject(new Error('timed out waiting for the debug runtime worker'));
        }, timeoutMs);
        const onMessage = message => {
            if (message.type === 'failure') {
                cleanup();
                reject(new Error(message.stack ?? message.message));
            } else if (predicate(message)) {
                cleanup();
                resolvePromise(message);
            }
        };
        const onError = error => {
            cleanup();
            reject(error);
        };
        const cleanup = () => {
            clearTimeout(timeout);
            worker.off('message', onMessage);
            worker.off('error', onError);
        };
        worker.on('message', onMessage);
        worker.on('error', onError);
    });
}
