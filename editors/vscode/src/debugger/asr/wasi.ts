import {
    closeSync,
    fstatSync,
    openSync,
    readSync,
} from 'node:fs';
import * as path from 'node:path';
import { randomFillSync } from 'node:crypto';

import type { RuntimeLogMessage } from '../runtimeProtocol';
import { GuestMemory } from './memory.ts';

const ERRNO_SUCCESS = 0;
const ERRNO_BADF = 8;
const ERRNO_EXIST = 20;
const ERRNO_INVAL = 28;
const ERRNO_IO = 29;
const ERRNO_ISDIR = 31;
const ERRNO_NOENT = 44;
const ERRNO_NOTDIR = 54;
const ERRNO_NOTCAPABLE = 76;
const PREOPEN_FD = 3;
const PREOPEN_NAME = '/mnt';
const encoder = new TextEncoder();

interface OpenFile {
    fd: number;
    offset: number;
}

export class WasiHost {
    private readonly memory: GuestMemory;
    private readonly scriptPath: string | undefined;
    private readonly log: (message: Omit<RuntimeLogMessage, 'type' | 'timestamp'>) => void;
    private readonly files = new Map<number, OpenFile>();
    private nextDescriptor = PREOPEN_FD + 1;
    private stderr = '';

    public constructor(
        memory: GuestMemory,
        scriptPath: string | undefined,
        log: (message: Omit<RuntimeLogMessage, 'type' | 'timestamp'>) => void,
    ) {
        this.memory = memory;
        this.scriptPath = scriptPath;
        this.log = log;
    }

    public imports(): Record<string, WebAssembly.ImportValue> {
        return {
            args_sizes_get: (countPointer: number, sizePointer: number) => {
                this.memory.writeU32(countPointer, 0);
                this.memory.writeU32(sizePointer, 0);
                return ERRNO_SUCCESS;
            },
            args_get: () => ERRNO_SUCCESS,
            environ_sizes_get: (countPointer: number, sizePointer: number) => {
                const environment = this.environment();
                this.memory.writeU32(countPointer, environment.length);
                this.memory.writeU32(
                    sizePointer,
                    environment.reduce((sum, value) => sum + encoder.encode(value).length + 1, 0),
                );
                return ERRNO_SUCCESS;
            },
            environ_get: (pointers: number, buffer: number) => {
                let cursor = buffer;
                for (const [index, value] of this.environment().entries()) {
                    const bytes = encoder.encode(value);
                    this.memory.writeU32(pointers + index * 4, cursor);
                    this.memory.writeBytes(cursor, bytes);
                    this.memory.writeU8(cursor + bytes.length, 0);
                    cursor += bytes.length + 1;
                }
                return ERRNO_SUCCESS;
            },
            clock_time_get: (clock: number, _precision: bigint, output: number) => {
                if (clock === 0) {
                    this.memory.writeU64(output, BigInt(Date.now()) * 1_000_000n);
                } else if (clock === 1) {
                    this.memory.writeU64(output, process.hrtime.bigint());
                } else {
                    return ERRNO_INVAL;
                }
                return ERRNO_SUCCESS;
            },
            random_get: (pointer: number, length: number) => {
                randomFillSync(this.memory.readBytes(pointer, length));
                return ERRNO_SUCCESS;
            },
            fd_prestat_get: (descriptor: number, output: number) => {
                if (descriptor !== PREOPEN_FD) return ERRNO_BADF;
                this.memory.writeBytes(output, new Uint8Array(8));
                this.memory.writeU32(output + 4, encoder.encode(PREOPEN_NAME).length);
                return ERRNO_SUCCESS;
            },
            fd_prestat_dir_name: (descriptor: number, pointer: number, length: number) => {
                if (descriptor !== PREOPEN_FD) return ERRNO_BADF;
                const bytes = encoder.encode(PREOPEN_NAME);
                if (length < bytes.length) return ERRNO_INVAL;
                this.memory.writeBytes(pointer, bytes);
                return ERRNO_SUCCESS;
            },
            path_open: (
                descriptor: number,
                _lookupFlags: number,
                pathPointer: number,
                pathLength: number,
                openFlags: number,
                rights: bigint,
                _inheritingRights: bigint,
                descriptorFlags: number,
                output: number,
            ) => this.open(
                descriptor,
                this.memory.readString(pathPointer, pathLength),
                openFlags,
                rights,
                descriptorFlags,
                output,
            ),
            fd_read: (descriptor: number, vectors: number, vectorCount: number, output: number) => {
                const file = this.files.get(descriptor);
                if (file === undefined) return ERRNO_BADF;
                let total = 0;
                try {
                    for (let index = 0; index < vectorCount; index += 1) {
                        const pointer = this.memory.readU32(vectors + index * 8);
                        const length = this.memory.readU32(vectors + index * 8 + 4);
                        const bytes = this.memory.readBytes(pointer, length);
                        const read = readSync(file.fd, bytes, 0, length, file.offset);
                        file.offset += read;
                        total += read;
                        if (read < length) break;
                    }
                    this.memory.writeU32(output, total);
                    return ERRNO_SUCCESS;
                } catch {
                    return ERRNO_IO;
                }
            },
            fd_seek: (descriptor: number, offset: bigint, whence: number, output: number) => {
                const file = this.files.get(descriptor);
                if (file === undefined) return ERRNO_BADF;
                let base: bigint;
                if (whence === 0) base = 0n;
                else if (whence === 1) base = BigInt(file.offset);
                else if (whence === 2) base = fstatSync(file.fd, { bigint: true }).size;
                else return ERRNO_INVAL;
                const position = base + offset;
                if (position < 0n || position > BigInt(Number.MAX_SAFE_INTEGER)) return ERRNO_INVAL;
                file.offset = Number(position);
                this.memory.writeU64(output, position);
                return ERRNO_SUCCESS;
            },
            fd_tell: (descriptor: number, output: number) => {
                const file = this.files.get(descriptor);
                if (file === undefined) return ERRNO_BADF;
                this.memory.writeU64(output, BigInt(file.offset));
                return ERRNO_SUCCESS;
            },
            fd_filestat_get: (descriptor: number, output: number) => {
                const file = this.files.get(descriptor);
                if (file === undefined) return ERRNO_BADF;
                try {
                    const stat = fstatSync(file.fd, { bigint: true });
                    this.memory.writeBytes(output, new Uint8Array(64));
                    this.memory.writeU64(output, stat.dev);
                    this.memory.writeU64(output + 8, stat.ino);
                    this.memory.writeU8(output + 16, stat.isFile() ? 4 : stat.isDirectory() ? 3 : 0);
                    this.memory.writeU64(output + 24, stat.nlink);
                    this.memory.writeU64(output + 32, stat.size);
                    this.memory.writeU64(output + 40, stat.atimeNs);
                    this.memory.writeU64(output + 48, stat.mtimeNs);
                    this.memory.writeU64(output + 56, stat.ctimeNs);
                    return ERRNO_SUCCESS;
                } catch {
                    return ERRNO_IO;
                }
            },
            fd_close: (descriptor: number) => this.close(descriptor),
            fd_write: (descriptor: number, vectors: number, vectorCount: number, output: number) => {
                let total = 0;
                for (let index = 0; index < vectorCount; index += 1) {
                    const pointer = this.memory.readU32(vectors + index * 8);
                    const length = this.memory.readU32(vectors + index * 8 + 4);
                    total += length;
                    if (descriptor === 2) {
                        this.stderr += new TextDecoder().decode(this.memory.readBytes(pointer, length));
                    }
                }
                this.memory.writeU32(output, total);
                this.flushStderr(false);
                return descriptor === 1 || descriptor === 2 ? ERRNO_SUCCESS : ERRNO_BADF;
            },
            proc_exit: (code: number) => {
                throw new WebAssembly.RuntimeError(`WASI process exited with code ${code}`);
            },
        };
    }

    public dispose(): void {
        for (const descriptor of [...this.files.keys()]) this.close(descriptor);
        this.flushStderr(true);
    }

    private environment(): string[] {
        return this.scriptPath === undefined
            ? []
            : [`SCRIPT_PATH=${nativePathToWasi(this.scriptPath)}`];
    }

    private open(
        descriptor: number,
        wasiPath: string,
        openFlags: number,
        rights: bigint,
        descriptorFlags: number,
        output: number,
    ): number {
        if (descriptor !== PREOPEN_FD) return ERRNO_BADF;
        const writeRight = 1n << 6n;
        const mutatingOpenFlags = 1 | 4 | 8;
        if ((rights & writeRight) !== 0n || (openFlags & mutatingOpenFlags) !== 0 || descriptorFlags !== 0) {
            return ERRNO_NOTCAPABLE;
        }
        let nativePath: string;
        try {
            nativePath = wasiRelativePathToNative(wasiPath);
        } catch {
            return ERRNO_NOTCAPABLE;
        }
        try {
            const fileDescriptor = openSync(nativePath, 'r');
            const stat = fstatSync(fileDescriptor);
            if (stat.isDirectory()) {
                closeSync(fileDescriptor);
                return ERRNO_ISDIR;
            }
            const opened = this.nextDescriptor++;
            this.files.set(opened, { fd: fileDescriptor, offset: 0 });
            this.memory.writeU32(output, opened);
            return ERRNO_SUCCESS;
        } catch (error) {
            const code = (error as NodeJS.ErrnoException).code;
            if (code === 'ENOENT') return ERRNO_NOENT;
            if (code === 'ENOTDIR') return ERRNO_NOTDIR;
            if (code === 'EEXIST') return ERRNO_EXIST;
            return ERRNO_IO;
        }
    }

    private close(descriptor: number): number {
        const file = this.files.get(descriptor);
        if (file === undefined) return ERRNO_BADF;
        this.files.delete(descriptor);
        try {
            closeSync(file.fd);
            return ERRNO_SUCCESS;
        } catch {
            return ERRNO_IO;
        }
    }

    private flushStderr(all: boolean): void {
        const lines = this.stderr.split('\n');
        this.stderr = lines.pop() ?? '';
        if (all && this.stderr.length > 0) {
            lines.push(this.stderr);
            this.stderr = '';
        }
        for (const line of lines) {
            this.log({ source: 'autoSplitter', level: 'info', message: line.replace(/\r$/, '') });
        }
    }
}

export function nativePathToWasi(nativePath: string): string {
    const resolved = path.resolve(nativePath);
    if (process.platform === 'win32') {
        const match = /^([a-zA-Z]):[\\/](.*)$/.exec(resolved);
        if (match === null) throw new Error(`Cannot expose ${nativePath} through /mnt.`);
        return `/mnt/${match[1].toLowerCase()}/${match[2].replaceAll('\\', '/')}`;
    }
    return `/mnt${resolved}`;
}

export function wasiRelativePathToNative(wasiPath: string): string {
    if (wasiPath.startsWith('/') || wasiPath.includes('\\')) {
        throw new Error('WASI preopen paths must be relative and use forward slashes.');
    }
    const segments = wasiPath.split('/').filter(segment => segment.length > 0);
    if (segments.some(segment => segment === '.' || segment === '..' || segment.includes('\0'))) {
        throw new Error('WASI path escapes the read-only preopen.');
    }
    if (process.platform === 'win32') {
        const [drive, ...rest] = segments;
        if (drive === undefined || !/^[a-zA-Z]$/.test(drive)) {
            throw new Error('A Windows WASI path must begin with a drive letter.');
        }
        return path.win32.join(`${drive}:\\`, ...rest);
    }
    return path.posix.join('/', ...segments);
}
