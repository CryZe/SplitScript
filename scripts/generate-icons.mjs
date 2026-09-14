import { readFile, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

// Rasterization is only needed when updating artwork, not during extension builds.
// Install sharp in the ignored target/icon-tools folder; see assets/README.md.
const require = createRequire(new URL('../target/icon-tools/package.json', import.meta.url));
const sharp = require('sharp');
const asset = path => new URL(`../${path}`, import.meta.url);

const color = await readFile(asset('assets/icon.svg'), 'utf8');
const monochrome = color
    .replace('White code braces surrounding a red, green, and blue stopwatch.',
        'Code braces surrounding a stopwatch, drawn in the current text color.')
    .replace(/#[\da-f]{6}/giu, 'currentColor');
const adaptive = color
    .replace('White code braces surrounding a red, green, and blue stopwatch.',
        'Code braces and stopwatch hands in the current text color, with a red, green, and blue ring.')
    .replaceAll('#FFFFFF', 'currentColor');
const light = adaptive.replaceAll('currentColor', '#1F2328');
const badge = color.replace('  <path',
    '  <rect width="256" height="256" fill="#171717"/>\n  <path');

await writeFile(asset('assets/icon-monochrome.svg'), monochrome);
await writeFile(asset('assets/icon-currentcolor.svg'), adaptive);
await writeFile(asset('assets/icon-light.svg'), light);
await writeFile(asset('editors/vscode/media/splitscript-debugger.svg'),
    monochrome
        .replace('width="256" height="256"', 'width="24" height="24"')
        .replace('viewBox="0 0 256 256"', 'viewBox="18 18 220 220"'));
await sharp(Buffer.from(badge))
    .resize(128, 128)
    .removeAlpha()
    .png()
    .toFile(fileURLToPath(asset('editors/vscode/media/icon.png')));
for (const [theme, artwork] of [['light', light], ['dark', color]]) {
    await sharp(Buffer.from(artwork))
        .resize(128, 128)
        .png()
        .toFile(fileURLToPath(asset(`editors/vscode/media/icon-readme-${theme}.png`)));
}

console.log('Generated transparent theme-aware README icons, currentColor SVGs, sidebar SVG, and extension PNG.');
