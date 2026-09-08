import { copyFile, mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(extension, '..', '..');

if (process.platform !== 'win32' || process.arch !== 'x64') {
    console.log(`Skipping native process bridge on unsupported build host ${process.platform}-${process.arch}.`);
    process.exit(0);
}

const result = spawnSync('cargo', [
    'build',
    '--release',
    '--package',
    'splitscript-process-native',
    '--lib',
    '--bin',
    'splitscript-process-fixture',
], {
    cwd: repository,
    stdio: 'inherit',
    shell: false,
});
if (result.error) {
    throw result.error;
}
if (result.status !== 0) {
    process.exit(result.status ?? 1);
}

const destination = resolve(
    extension,
    'dist',
    'native',
    'win32-x64',
    'splitscript_process_native.node',
);
await mkdir(dirname(destination), { recursive: true });
await copyFile(
    resolve(repository, 'target', 'release', 'splitscript_process_native.dll'),
    destination,
);
console.log(`Copied native process bridge to ${destination}`);
