import { copyFile, mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(extension, '..', '..');
const production = process.argv.includes('--production');
const profile = production ? 'max-opt' : 'release';
const outputOption = process.argv.indexOf('--output');
if (outputOption >= 0 && process.argv[outputOption + 1] === undefined) {
    throw new Error('--output requires a directory');
}
const output = resolve(
    outputOption >= 0 ? process.argv[outputOption + 1] : resolve(extension, 'dist'),
);
process.env.SPLITSCRIPT_VSCODE_DIST = output;
process.env.SPLITSCRIPT_NATIVE_OUTPUT_ROOT = resolve(output, 'native');

// The package audit stages an already-built production tree in a temporary
// directory. VSCE always invokes `vscode:prepublish`, even for that immutable
// staging tree; acknowledge it without rebuilding or cleaning the artifacts
// whose package manifest is being verified.
if (process.env.SPLITSCRIPT_PACKAGE_PREBUILT === '1') {
    console.log('Using the prebuilt extension staging tree.');
    process.exit(0);
}

run(process.execPath, [resolve(extension, 'scripts', 'clean.mjs')]);
run(process.execPath, [
    resolve(extension, 'node_modules', 'typescript', 'bin', 'tsc'),
    '-p',
    resolve(extension, 'tsconfig.node.json'),
    '--noEmit',
]);
run(process.execPath, [
    resolve(extension, 'scripts', 'bundle-node.mjs'),
    ...(production ? ['--production'] : []),
]);
run(process.execPath, [
    resolve(extension, 'scripts', 'bundle-web.mjs'),
    ...(production ? ['--production'] : []),
]);
run('cargo', [
    'build',
    '--manifest-path',
    resolve(repository, 'Cargo.toml'),
    '--profile',
    profile,
    '--target',
    'wasm32-unknown-unknown',
    '--package',
    'splitscript-vscode-wasm',
]);

const source = resolve(
    repository,
    'target',
    'wasm32-unknown-unknown',
    profile,
    'splitscript_vscode_wasm.wasm',
);
const destination = resolve(output, 'splitscript_vscode_wasm.wasm');
await mkdir(dirname(destination), { recursive: true });
await copyFile(source, destination);

run(process.execPath, [resolve(extension, 'scripts', 'build-native.mjs')]);

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
