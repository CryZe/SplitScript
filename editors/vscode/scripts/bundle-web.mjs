import { build } from 'esbuild';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const production = process.argv.includes('--production');
const common = {
    bundle: true,
    logLevel: 'info',
    minify: production,
    sourcemap: production ? false : 'linked',
    target: 'es2022',
};

await build({
    ...common,
    entryPoints: [resolve(extension, 'src', 'extensionBrowser.ts')],
    outfile: resolve(extension, 'dist', 'web', 'extension.js'),
    platform: 'browser',
    format: 'cjs',
    external: ['vscode'],
    mainFields: ['browser', 'module', 'main'],
});

for (const [source, output] of [
    ['embeddedCompilerBrowserWorker.ts', 'embeddedCompilerWorker.js'],
    ['embeddedLanguageServerBrowserWorker.ts', 'embeddedLanguageServerWorker.js'],
]) {
    await build({
        ...common,
        entryPoints: [resolve(extension, 'src', source)],
        outfile: resolve(extension, 'dist', 'web', output),
        platform: 'browser',
        format: 'iife',
        mainFields: ['browser', 'module', 'main'],
    });
}

if (!production) {
    await build({
        ...common,
        entryPoints: [resolve(extension, 'test', 'webHost.ts')],
        outfile: resolve(extension, 'dist', 'web', 'test', 'index.js'),
        platform: 'browser',
        format: 'cjs',
        external: ['vscode'],
        mainFields: ['browser', 'module', 'main'],
    });
}
