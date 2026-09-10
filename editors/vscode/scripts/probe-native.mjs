import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';

const supportedPlatforms = new Set([
    'win32-x64',
    'linux-x64',
    'linux-arm64',
    'darwin-x64',
    'darwin-arm64',
]);
const platform = `${process.platform}-${process.arch}`;
if (!supportedPlatforms.has(platform)) {
    console.log(`Skipping native process probe on unsupported host ${platform}.`);
    process.exit(0);
}

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(extension, '..', '..');
const nativeRoot = resolve(
    process.env.SPLITSCRIPT_NATIVE_OUTPUT_ROOT ?? resolve(extension, 'dist', 'native'),
);
const require = createRequire(import.meta.url);
const native = require(resolve(
    nativeRoot,
    platform,
    'splitscript_process_native.node',
));
const fixtureName = process.platform === 'win32'
    ? 'splitscript-process-fixture.exe'
    : 'splitscript-process-fixture';
const fixture = spawn(
    resolve(repository, 'target', 'release', fixtureName),
    [],
    { stdio: ['pipe', 'pipe', 'inherit'] },
);

try {
    const line = await firstLine(fixture.stdout);
    const fields = Object.fromEntries(line.split(';').map(field => field.split('=', 2)));
    const pid = Number(fields.pid);
    const length = Number(fields.length);
    assert(Number.isInteger(pid) && pid > 0);
    assert(Number.isInteger(length) && length > 0);

    assert(native.listProcessesByName(fixtureName).includes(pid));

    const handle = native.attachByPid(pid);
    assert.equal(native.processId(handle), pid);
    assert.match(native.processPath(handle), new RegExp(`${escapeRegExp(fixtureName)}$`, 'i'));
    assert.equal(native.isOpen(handle), true);
    assert(BigInt(native.moduleAddress(handle, fixtureName)) > 0n);
    assert(BigInt(native.moduleSize(handle, fixtureName)) > 0n);
    assert.match(
        native.modulePath(handle, fixtureName),
        new RegExp(`${escapeRegExp(fixtureName)}$`, 'i'),
    );
    const rangeCount = native.memoryRangeCount(handle);
    assert(rangeCount > 0);
    assert(BigInt(native.memoryRangeAddress(handle, 0)) > 0n);
    assert(BigInt(native.memoryRangeSize(handle, 0)) > 0n);
    assert(BigInt(native.memoryRangeFlags(handle, 0)) > 0n);
    const actual = native.readProcessMemory(handle, fields.address, length);
    assert.equal(Buffer.from(actual).toString('utf8'), fields.expected);
    assert.equal(native.detach(handle), true);
    assert.equal(native.detach(handle), false);
    const namedHandle = native.attachByName(fixtureName);
    assert.equal(native.processId(namedHandle), pid);
    assert.equal(native.detach(namedHandle), true);
    console.log(
        `Native ${platform} process probe passed: discovery, modules, ranges, and ${length}-byte read from PID ${pid}.`,
    );
} finally {
    fixture.stdin.end('\n');
    await new Promise(resolvePromise => fixture.once('exit', resolvePromise));
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

function escapeRegExp(value) {
    return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
