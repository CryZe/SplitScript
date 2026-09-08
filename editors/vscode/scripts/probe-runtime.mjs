import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { Worker } from 'node:worker_threads';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(extension, '..', '..');
const temporary = await mkdtemp(join(tmpdir(), 'splitscript-runtime-probe-'));

try {
    const generatedWasm = resolve(temporary, 'runtime-probe.wasm');
    run('cargo', [
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
    ], repository);

    const workerPath = resolve(extension, 'dist', 'runtimeProbeWorker.js');
    const generatedResult = await runWorker(workerPath, await readFile(generatedWasm), true);
    assert.equal(generatedResult.updated, true);
    assert(generatedResult.ready.imports.includes('env.process_attach'));
    assert(generatedResult.ready.memoryBytes > 0);

    const infiniteWasm = resolve(temporary, 'infinite-update.wasm');
    run('wasm-tools', [
        'parse',
        resolve(extension, 'test', 'fixtures', 'infinite-update.wat'),
        '--output',
        infiniteWasm,
    ], repository);
    const infinite = new Worker(workerPath);
    try {
        const ready = waitForMessage(infinite, 'ready', 5_000);
        const bytes = await readFile(infiniteWasm);
        infinite.postMessage({
            wasm: exactArrayBuffer(bytes),
            runUpdate: true,
        });
        await ready;
        const started = performance.now();
        await infinite.terminate();
        assert(performance.now() - started < 1_000, 'terminating a hung runtime worker took too long');
    } finally {
        await infinite.terminate();
    }

    console.log(
        `Runtime probe passed: ${generatedResult.ready.imports.length} imports, `
        + `${generatedResult.ready.memoryBytes} Wasm memory bytes, hung worker terminated.`,
    );
} finally {
    await rm(temporary, { recursive: true, force: true });
}

async function runWorker(workerPath, wasm, runUpdate) {
    const worker = new Worker(workerPath);
    try {
        const readyPromise = waitForMessage(worker, 'ready', 5_000);
        const updatedPromise = runUpdate
            ? waitForMessage(worker, 'updated', 5_000)
            : Promise.resolve(undefined);
        worker.postMessage({ wasm: exactArrayBuffer(wasm), runUpdate });
        const ready = await readyPromise;
        await updatedPromise;
        return { ready, updated: runUpdate };
    } finally {
        await worker.terminate();
    }
}

function waitForMessage(worker, type, timeoutMs) {
    return new Promise((resolvePromise, reject) => {
        const timeout = setTimeout(() => {
            cleanup();
            reject(new Error(`timed out waiting for runtime worker message ${type}`));
        }, timeoutMs);
        const onMessage = message => {
            if (message.type === 'failure') {
                cleanup();
                reject(new Error(message.stack ?? message.message));
            } else if (message.type === type) {
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

function exactArrayBuffer(buffer) {
    return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

function run(command, arguments_, cwd) {
    const result = spawnSync(command, arguments_, { cwd, stdio: 'inherit', shell: false });
    if (result.error) {
        throw result.error;
    }
    assert.equal(result.status, 0, `${command} exited with ${result.status}`);
}
