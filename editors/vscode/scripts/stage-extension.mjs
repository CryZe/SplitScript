import { copyFile, cp, mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';

const staticPackagePaths = Object.freeze([
    'package.json',
    'README.md',
    'language-configuration.json',
    'syntaxes',
    'styles',
    'media',
]);

export async function stageExtension(extension, dist, destination, includeBuildScript = false) {
    await mkdir(destination, { recursive: true });
    for (const path of staticPackagePaths) {
        await cp(resolve(extension, path), resolve(destination, path), { recursive: true });
    }
    await cp(dist, resolve(destination, 'dist'), { recursive: true });

    if (includeBuildScript) {
        const stagedBuild = resolve(destination, 'scripts', 'build.mjs');
        await mkdir(dirname(stagedBuild), { recursive: true });
        await copyFile(resolve(extension, 'scripts', 'build.mjs'), stagedBuild);
    }
}
