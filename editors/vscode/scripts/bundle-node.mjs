import { build } from 'esbuild';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const outputDirectory = resolve(process.env.SPLITSCRIPT_VSCODE_DIST ?? resolve(extension, 'dist'));
const production = process.argv.includes('--production');
const common = {
    bundle: true,
    logLevel: 'info',
    minify: production,
    platform: 'node',
    format: 'cjs',
    sourcemap: production ? false : 'linked',
    target: 'node20',
};

await build({
    ...common,
    entryPoints: [resolve(extension, 'src', 'extension.ts')],
    outfile: resolve(outputDirectory, 'extension.js'),
    external: ['vscode'],
});

for (const [source, output] of [
    ['embeddedCompilerNodeWorker.ts', 'embeddedCompilerNodeWorker.js'],
    ['embeddedLanguageServerNodeWorker.ts', 'embeddedLanguageServerNodeWorker.js'],
    ['debugger/runtimeWorker.ts', 'runtimeWorker.js'],
    ['debugger/runtimeProbeWorker.ts', 'runtimeProbeWorker.js'],
]) {
    await build({
        ...common,
        entryPoints: [resolve(extension, 'src', source)],
        outfile: resolve(outputDirectory, output),
    });
}

if (!production) {
    await build({
        ...common,
        entryPoints: [resolve(extension, 'src', 'embeddedCompilerWorkerClient.ts')],
        outfile: resolve(outputDirectory, 'embeddedCompilerWorkerClient.js'),
    });
}
