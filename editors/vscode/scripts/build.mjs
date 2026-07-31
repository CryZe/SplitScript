import { copyFile, mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(extension, '..', '..');
const production = process.argv.includes('--production');

run(process.execPath, [resolve(extension, 'scripts', 'clean.mjs')]);
run(process.execPath, [
    resolve(extension, 'node_modules', 'typescript', 'bin', 'tsc'),
    '-p',
    resolve(extension, 'tsconfig.node.json'),
    ...(production ? ['--sourceMap', 'false'] : []),
]);
run(process.execPath, [
    resolve(extension, 'scripts', 'bundle-web.mjs'),
    ...(production ? ['--production'] : []),
]);
run('cargo', [
    'build',
    '--manifest-path',
    resolve(repository, 'Cargo.toml'),
    '--release',
    '--target',
    'wasm32-unknown-unknown',
    '--package',
    'splitscript-vscode-wasm',
]);

const source = resolve(
    repository,
    'target',
    'wasm32-unknown-unknown',
    'release',
    'splitscript_vscode_wasm.wasm',
);
const destination = resolve(extension, 'dist', 'splitscript_vscode_wasm.wasm');
await mkdir(dirname(destination), { recursive: true });
await copyFile(source, destination);

function run(command, arguments_) {
    const result = spawnSync(command, arguments_, {
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
}
