import { rm } from 'node:fs/promises';
import { basename, dirname, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(extension, '..', '..');
const defaultOutput = resolve(extension, 'dist');
const output = resolve(process.env.SPLITSCRIPT_VSCODE_DIST ?? defaultOutput);
const target = resolve(repository, 'target');
if (output !== defaultOutput && !output.startsWith(`${target}${sep}`)) {
    throw new Error(`refusing to clean unexpected extension output path: ${output}`);
}
if (output === defaultOutput && (dirname(output) !== extension || basename(output) !== 'dist')) {
    throw new Error(`refusing to clean unexpected extension output path: ${output}`);
}
await rm(output, { recursive: true, force: true });
