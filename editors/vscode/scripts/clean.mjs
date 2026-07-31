import { rm } from 'node:fs/promises';
import { basename, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const output = resolve(extension, 'dist');
if (dirname(output) !== extension || basename(output) !== 'dist') {
    throw new Error(`refusing to clean unexpected extension output path: ${output}`);
}
await rm(output, { recursive: true, force: true });
