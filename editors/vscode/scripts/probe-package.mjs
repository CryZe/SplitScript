import assert from 'node:assert/strict';
import { mkdtemp, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

if (process.platform !== 'win32' || process.arch !== 'x64') {
    console.log(`Skipping Windows x64 VSIX native-artifact probe on ${process.platform}-${process.arch}.`);
    process.exit(0);
}

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const vsce = resolve(extension, 'node_modules', '@vscode', 'vsce', 'vsce');
const listing = spawnSync(process.execPath, [vsce, 'ls', '--no-dependencies'], {
    cwd: extension,
    encoding: 'utf8',
    shell: false,
});
if (listing.error) {
    throw listing.error;
}
assert.equal(listing.status, 0, listing.stderr);
assert.match(
    listing.stdout.replaceAll('\\', '/'),
    /dist\/native\/win32-x64\/splitscript_process_native\.node/,
    'the native process bridge is missing from the extension package file list',
);

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
    if (packaged.error) {
        throw packaged.error;
    }
    assert.equal(packaged.status, 0, packaged.stderr || packaged.stdout);
    assert((await stat(output)).size > 0);
    console.log('VSIX packaging probe passed with the Windows x64 native bridge included.');
} finally {
    await rm(temporary, { recursive: true, force: true });
}
