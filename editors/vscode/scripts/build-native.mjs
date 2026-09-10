import { copyFile, mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(extension, '..', '..');
const fileName = 'splitscript_process_native.node';
const outputRoot = resolve(
    process.env.SPLITSCRIPT_NATIVE_OUTPUT_ROOT ?? resolve(extension, 'dist', 'native'),
);
const platforms = {
    'win32-x64': 'splitscript_process_native.dll',
    'linux-x64': 'libsplitscript_process_native.so',
    'linux-arm64': 'libsplitscript_process_native.so',
    'darwin-x64': 'libsplitscript_process_native.dylib',
    'darwin-arm64': 'libsplitscript_process_native.dylib',
};

const prebuiltRoot = process.env.SPLITSCRIPT_NATIVE_ARTIFACTS;
if (prebuiltRoot === undefined) {
    await buildHostArtifact();
} else {
    await copyPrebuiltArtifacts(resolve(prebuiltRoot));
}

async function buildHostArtifact() {
    const platform = `${process.platform}-${process.arch}`;
    const library = platforms[platform];
    if (library === undefined) {
        console.log(`Skipping native process bridge on unsupported build host ${platform}.`);
        return;
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
    if (result.error) throw result.error;
    if (result.status !== 0) process.exit(result.status ?? 1);

    const destination = resolve(outputRoot, platform, fileName);
    await mkdir(dirname(destination), { recursive: true });
    await copyFile(resolve(repository, 'target', 'release', library), destination);
    console.log(`Copied native process bridge to ${destination}`);
}

async function copyPrebuiltArtifacts(root) {
    for (const platform of requiredPlatforms()) {
        if (!(platform in platforms)) {
            throw new Error(`Unsupported prebuilt native platform ${platform}.`);
        }
        const source = resolve(root, platform, fileName);
        const destination = resolve(outputRoot, platform, fileName);
        await mkdir(dirname(destination), { recursive: true });
        await copyFile(source, destination);
        console.log(`Copied prebuilt native process bridge for ${platform}.`);
    }
}

function requiredPlatforms() {
    const configured = process.env.SPLITSCRIPT_REQUIRED_NATIVE_PLATFORMS;
    return configured === undefined
        ? Object.keys(platforms)
        : configured.split(',').map(value => value.trim()).filter(Boolean);
}
