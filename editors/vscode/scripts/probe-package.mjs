import assert from 'node:assert/strict';
import { mkdtemp, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const vsce = resolve(extension, 'node_modules', '@vscode', 'vsce', 'vsce');
const requiredPlatforms = (process.env.SPLITSCRIPT_REQUIRED_NATIVE_PLATFORMS
    ?? `${process.platform}-${process.arch}`)
    .split(',')
    .map(value => value.trim())
    .filter(Boolean);
const listing = spawnSync(process.execPath, [vsce, 'ls', '--no-dependencies'], {
    cwd: extension,
    encoding: 'utf8',
    shell: false,
});
if (listing.error) throw listing.error;
assert.equal(listing.status, 0, listing.stderr);
const files = listing.stdout.replaceAll('\\', '/');
for (const platform of requiredPlatforms) {
    assert.match(
        files,
        new RegExp(`dist/native/${escapeRegExp(platform)}/splitscript_process_native\\.node`),
        `the ${platform} native process bridge is missing from the extension package file list`,
    );
}

const temporary = await mkdtemp(join(tmpdir(), 'splitscript-vsix-probe-'));
try {
    const output = resolve(temporary, 'splitscript-probe.vsix');
    const packaged = spawnSync(process.execPath, [
        vsce,
        'package',
        '--no-dependencies',
        '--out',
        output,
    ], {
        cwd: extension,
        encoding: 'utf8',
        shell: false,
    });
    if (packaged.error) throw packaged.error;
    assert.equal(packaged.status, 0, packaged.stderr || packaged.stdout);
    assert((await stat(output)).size > 0);
    console.log(`VSIX packaging probe passed with ${requiredPlatforms.length} native bridge artifact(s).`);
} finally {
    await rm(temporary, { recursive: true, force: true });
}

function escapeRegExp(value) {
    return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
