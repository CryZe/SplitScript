import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { stageExtension } from './stage-extension.mjs';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const dist = resolve(process.argv[2] ?? resolve(extension, 'dist'));
const testWeb = resolve(
    extension,
    'node_modules',
    '@vscode',
    'test-web',
    'out',
    'server',
    'index.js',
);
const temporary = await mkdtemp(join(tmpdir(), 'splitscript-web-host-'));

try {
    const staging = resolve(temporary, 'extension');
    await stageExtension(extension, dist, staging);
    const result = spawnSync(process.execPath, [
        testWeb,
        '--browser=chromium',
        '--headless=true',
        '--quality=stable',
        '--commit=e4c7e7b1d6d060162f4aa7f8225271b67ce1df75',
        `--extensionDevelopmentPath=${staging}`,
        `--extensionTestsPath=${resolve(staging, 'dist', 'web', 'test', 'index.js')}`,
        resolve(extension, 'test-workspace'),
    ], {
        cwd: extension,
        encoding: 'utf8',
        shell: false,
    });
    if (result.error) throw result.error;
    assert.equal(result.status, 0, result.stderr || result.stdout);
    process.stdout.write(result.stdout);
    process.stderr.write(result.stderr);
} finally {
    await rm(temporary, { recursive: true, force: true });
}
