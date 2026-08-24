import { build } from 'esbuild';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
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
    outfile: resolve(extension, 'dist', 'extension.js'),
    external: ['vscode'],
});

for (const [source, output] of [
    ['embeddedCompilerNodeWorker.ts', 'embeddedCompilerNodeWorker.js'],
    ['embeddedLanguageServerNodeWorker.ts', 'embeddedLanguageServerNodeWorker.js'],
]) {
    await build({
        ...common,
        entryPoints: [resolve(extension, 'src', source)],
        outfile: resolve(extension, 'dist', output),
    });
}
