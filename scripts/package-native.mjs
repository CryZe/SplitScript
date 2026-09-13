import { spawnSync } from 'node:child_process';
import {
    chmodSync,
    copyFileSync,
    mkdirSync,
    rmSync,
    statSync,
} from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const platform = process.argv[2];
const supportedPlatforms = new Map([
    ['windows-x64', ['win32', 'x64']],
    ['linux-x64', ['linux', 'x64']],
    ['linux-arm64', ['linux', 'arm64']],
    ['macos-x64', ['darwin', 'x64']],
    ['macos-arm64', ['darwin', 'arm64']],
]);

if (!supportedPlatforms.has(platform)) {
    throw new Error(
        `usage: node scripts/package-native.mjs <${[...supportedPlatforms.keys()].join('|')}>`,
    );
}

const [expectedPlatform, expectedArchitecture] = supportedPlatforms.get(platform);
if (process.platform !== expectedPlatform || process.arch !== expectedArchitecture) {
    throw new Error(
        `refusing to label ${process.platform}-${process.arch} binaries as ${platform}`,
    );
}

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const executableSuffix = process.platform === 'win32' ? '.exe' : '';
const binaryDirectory = join(repository, 'target', 'max-opt');
const splitc = join(binaryDirectory, `splitc${executableSuffix}`);
const splitls = join(binaryDirectory, `splitls${executableSuffix}`);

function run(command, args, options = {}) {
    const result = spawnSync(command, args, {
        cwd: repository,
        encoding: 'utf8',
        ...options,
    });
    if (result.error) {
        throw result.error;
    }
    if (result.status !== 0) {
        throw new Error(
            `${command} ${args.join(' ')} failed with status ${result.status}\n`
            + `${result.stdout ?? ''}${result.stderr ?? ''}`,
        );
    }
    return result;
}

const compilerVersion = run(splitc, ['--version']).stdout.trim();
if (!compilerVersion.startsWith('splitc ')) {
    throw new Error(`unexpected splitc version output: ${compilerVersion}`);
}
const expectedRevision = process.env.GITHUB_SHA?.slice(0, 12);
if (expectedRevision && !compilerVersion.endsWith(`(${expectedRevision})`)) {
    throw new Error(
        `splitc identifies itself as ${compilerVersion}, expected revision ${expectedRevision}`,
    );
}

// Closing standard input is a protocol-level smoke test: splitls must start,
// initialize its language-server state, observe a clean EOF, and exit.
run(splitls, [], { input: '' });

const packageName = `splitscript-${platform}`;
const stagingRoot = join(repository, 'target', 'native-package');
const packageDirectory = join(stagingRoot, packageName);
const releaseDirectory = join(repository, 'target', 'release-assets');
const archiveExtension = process.platform === 'win32' ? '.zip' : '.tar.gz';
const archive = join(releaseDirectory, `${packageName}${archiveExtension}`);

rmSync(packageDirectory, { recursive: true, force: true });
mkdirSync(packageDirectory, { recursive: true });
mkdirSync(releaseDirectory, { recursive: true });
rmSync(archive, { force: true });

for (const source of [splitc, splitls]) {
    const destination = join(packageDirectory, basename(source));
    copyFileSync(source, destination);
    if (process.platform !== 'win32') {
        chmodSync(destination, 0o755);
    }
}
copyFileSync(
    join(repository, 'docs', 'INSTALLATION.md'),
    join(packageDirectory, 'INSTALLATION.md'),
);

const createArchiveArguments = process.platform === 'win32'
    ? ['-a', '-cf', archive, '-C', stagingRoot, packageName]
    : ['-czf', archive, '-C', stagingRoot, packageName];
run('tar', createArchiveArguments);
if (statSync(archive).size === 0) {
    throw new Error(`native archive is empty: ${archive}`);
}

const listArchiveArguments = process.platform === 'win32'
    ? ['-tf', archive]
    : ['-tzf', archive];
const entries = run('tar', listArchiveArguments).stdout
    .split(/\r?\n/u)
    .filter(Boolean)
    .map((entry) => entry.replaceAll('\\', '/'));
for (const expected of [
    `${packageName}/splitc${executableSuffix}`,
    `${packageName}/splitls${executableSuffix}`,
    `${packageName}/INSTALLATION.md`,
]) {
    if (!entries.includes(expected)) {
        throw new Error(`native archive is missing ${expected}`);
    }
}

console.log(JSON.stringify({ archive, compilerVersion, entries }));
