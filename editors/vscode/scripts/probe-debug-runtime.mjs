import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn, spawnSync } from 'node:child_process';
import { createInterface } from 'node:readline';
import { Worker } from 'node:worker_threads';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(extension, '..', '..');
const temporary = await mkdtemp(join(tmpdir(), 'splitscript-debug-runtime-'));
const runtimeWorker = process.env.SPLITSCRIPT_RUNTIME_WORKER_PATH
    ?? resolve(extension, 'dist', 'runtimeWorker.js');
const worker = new Worker(runtimeWorker);
const wasiWorker = new Worker(runtimeWorker);
let snapshotCount = 0;
worker.on('message', message => {
    if (message.type === 'snapshot') snapshotCount += 1;
});
const fixture = process.platform === 'win32' && process.arch === 'x64'
    ? spawn(
        resolve(repository, 'target', 'release', 'splitscript-process-fixture.exe'),
        [],
        { stdio: ['pipe', 'pipe', 'inherit'] },
    )
    : undefined;

try {
    const fixtureFields = fixture === undefined
        ? undefined
        : Object.fromEntries((await firstLine(fixture.stdout)).split(';').map(field => field.split('=', 2)));
    const generatedWasm = resolve(temporary, 'runtime-probe.wasm');
    const probeSource = resolve(temporary, 'runtime-probe.split');
    let source = await readFile(resolve(extension, 'test', 'fixtures', 'runtime-probe.split'), 'utf8');
    if (fixtureFields !== undefined) {
        source = source.replace(
            'state "splitscript-process-fixture.exe" {}',
            `state "splitscript-process-fixture.exe" { marker: u8 at ${fixtureFields.address}; }`,
        );
        source += '\nstart { return current.marker == 83 }\n';
        source += 'whileAttached { setVariable("Marker", `{current.marker}`) }\n';
    }
    await writeFile(probeSource, source, 'utf8');
    const compiled = spawnSync('cargo', [
        'run',
        '--quiet',
        '--bin',
        'splitc',
        '--',
        probeSource,
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
            && message.snapshot.tickCount > 0,
    );
    const setupLog = waitFor(
        worker,
        message => message.type === 'log'
            && message.source === 'autoSplitter'
            && message.message === 'SplitScript runtime worker ready',
    );
    const attachedLog = fixture === undefined ? undefined : waitFor(
        worker,
        message => message.type === 'log'
            && message.source === 'autoSplitter'
            && message.message === 'SplitScript runtime worker probe attached',
    );
    const processRead = fixture === undefined ? undefined : waitFor(
        worker,
        message => message.type === 'snapshot'
            && message.snapshot.processes.some(process => process.pid === Number(fixtureFields.pid))
            && message.snapshot.timer.state === 'running',
    );
    const bytes = await readFile(generatedWasm);
    worker.postMessage({
        type: 'launch',
        wasm: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength),
        program: generatedWasm,
        scriptPath: probeSource,
        nativeModulePath: fixture === undefined ? undefined : resolve(
            extension,
            'dist',
            'native',
            'win32-x64',
            'splitscript_process_native.node',
        ),
    });
    const readyMessage = await ready;
    if (fixture === undefined) {
        assert(readyMessage.unsupportedImports.includes('env.process_attach'));
    } else {
        assert(!readyMessage.unsupportedImports.some(name => name.startsWith('env.process_')));
    }
    assert(!readyMessage.unsupportedImports.some(name => name.includes('settings')));
    await setupLog;
    await attachedLog;
    await processRead;
    snapshotCount = 0;
    await delay(750);
    assert(
        snapshotCount <= 6,
        `runtime emitted ${snapshotCount} snapshots in 750 ms while variables changed at 120 Hz`,
    );
    const running = await configured;
    assert(running.snapshot.memoryBytes > 0);
    assert.equal(running.snapshot.tickRateHz, 120);
    assert(running.snapshot.sampledTickCount > 0);
    assert(running.snapshot.retainedTickCount > 0);
    assert.equal(running.snapshot.settings.widgets.length, 5);

    const memoryDump = waitFor(
        worker,
        message => message.type === 'memoryDump' && message.requestId === 1,
    );
    worker.postMessage({ type: 'dumpMemory', requestId: 1 });
    const dumped = await memoryDump;
    assert(dumped.bytes.byteLength > 0);

    const resetStatistics = waitFor(
        worker,
        message => message.type === 'snapshot'
            && message.snapshot.sampledTickCount === 0,
    );
    worker.postMessage({ type: 'resetStatistics' });
    await resetStatistics;

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

    console.log('Production debug runtime probe passed: launch, process attach/read, tick statistics, memory dump, log, settings, WASI, timer controls.');
} finally {
    await stopWorker(worker);
    await stopWorker(wasiWorker);
    if (fixture !== undefined) {
        fixture.stdin.end('\n');
        await new Promise(resolvePromise => fixture.once('exit', resolvePromise));
    }
    await rm(temporary, { recursive: true, force: true });
}

async function stopWorker(worker) {
    const stopped = waitFor(worker, message => message.type === 'stopped');
    worker.postMessage({ type: 'shutdown' });
    await stopped;
    await worker.terminate();
}

async function firstLine(stream) {
    const lines = createInterface({ input: stream, crlfDelay: Infinity });
    try {
        return await new Promise((resolvePromise, reject) => {
            lines.once('line', resolvePromise);
            lines.once('error', reject);
        });
    } finally {
        lines.close();
    }
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

function delay(milliseconds) {
    return new Promise(resolvePromise => setTimeout(resolvePromise, milliseconds));
}
