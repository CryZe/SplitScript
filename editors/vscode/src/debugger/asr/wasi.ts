import { randomFillSync } from 'node:crypto';
import {
    closeSync,
    fstatSync,
    lstatSync,
    openSync,
    readlinkSync,
    readSync,
    readdirSync,
    statSync,
} from 'node:fs';
import * as path from 'node:path';

import type { RuntimeLogMessage } from '../runtimeProtocol';
import { GuestMemory } from './memory.ts';

const ERRNO_SUCCESS = 0;
const ERRNO_ACCES = 2;
const ERRNO_BADF = 8;
const ERRNO_EXIST = 20;
const ERRNO_INVAL = 28;
const ERRNO_IO = 29;
const ERRNO_ISDIR = 31;
const ERRNO_LOOP = 32;
const ERRNO_NAMETOOLONG = 37;
const ERRNO_NOENT = 44;
const ERRNO_NOSYS = 52;
const ERRNO_NOTDIR = 54;
const ERRNO_NOTEMPTY = 55;
const ERRNO_NOTCAPABLE = 76;

const FILETYPE_UNKNOWN = 0;
const FILETYPE_CHARACTER_DEVICE = 2;
const FILETYPE_DIRECTORY = 3;
const FILETYPE_REGULAR_FILE = 4;
const FILETYPE_SYMBOLIC_LINK = 7;

const RIGHT_FD_READ = 1n << 1n;
const RIGHT_FD_SEEK = 1n << 2n;
const RIGHT_FD_TELL = 1n << 5n;
const RIGHT_FD_WRITE = 1n << 6n;
const RIGHT_FD_ADVISE = 1n << 7n;
const RIGHT_PATH_OPEN = 1n << 13n;
const RIGHT_FD_READDIR = 1n << 14n;
const RIGHT_PATH_READLINK = 1n << 15n;
const RIGHT_PATH_FILESTAT_GET = 1n << 18n;
const RIGHT_FD_FILESTAT_GET = 1n << 21n;
const MUTATING_FILE_RIGHTS = RIGHT_FD_WRITE
    | (1n << 8n)
    | (1n << 22n)
    | (1n << 23n);
const PREOPEN_BASE_RIGHTS = RIGHT_PATH_OPEN
    | RIGHT_FD_READDIR
    | RIGHT_PATH_READLINK
    | RIGHT_PATH_FILESTAT_GET
    | RIGHT_FD_FILESTAT_GET;
const PREOPEN_INHERITING_RIGHTS = PREOPEN_BASE_RIGHTS
    | RIGHT_FD_READ
    | RIGHT_FD_SEEK
    | RIGHT_FD_TELL
    | RIGHT_FD_ADVISE;

const PREOPEN_FD = 3;
const PREOPEN_NAME = '/mnt';
const encoder = new TextEncoder();
const decoder = new TextDecoder();

interface OpenResource {
    nativePath: string;
    fd?: number;
    offset: number;
    directory: boolean;
    rightsBase: bigint;
    rightsInheriting: bigint;
}

interface WasiStat {
    dev: bigint;
    ino: bigint;
    nlink: bigint;
    size: bigint;
    atimeNs: bigint;
    mtimeNs: bigint;
    ctimeNs: bigint;
    isFile(): boolean;
    isDirectory(): boolean;
    isSymbolicLink(): boolean;
}

export const WASI_PREVIEW1_IMPORTS = [
    'args_get', 'args_sizes_get', 'clock_res_get', 'clock_time_get',
    'environ_get', 'environ_sizes_get', 'fd_advise', 'fd_allocate',
    'fd_close', 'fd_datasync', 'fd_fdstat_get', 'fd_fdstat_set_flags',
    'fd_fdstat_set_rights', 'fd_filestat_get', 'fd_filestat_set_size',
    'fd_filestat_set_times', 'fd_pread', 'fd_prestat_dir_name',
    'fd_prestat_get', 'fd_pwrite', 'fd_read', 'fd_readdir', 'fd_renumber',
    'fd_seek', 'fd_sync', 'fd_tell', 'fd_write', 'path_create_directory',
    'path_filestat_get', 'path_filestat_set_times', 'path_link', 'path_open',
    'path_readlink', 'path_remove_directory', 'path_rename', 'path_symlink',
    'path_unlink_file', 'poll_oneoff', 'proc_exit', 'proc_raise', 'random_get',
    'sched_yield', 'sock_accept', 'sock_recv', 'sock_send', 'sock_shutdown',
] as const;

export class WasiExit extends Error {
    public readonly code: number;

    public constructor(code: number) {
        super(`WASI process exited with code ${code}`);
        this.name = 'WasiExit';
        this.code = code;
    }
}

class WasiPathError extends Error {}

/** A hermetic, read-only implementation of WASI snapshot preview1 (WASI 0.1). */
export class WasiHost {
    private readonly memory: GuestMemory;
    private readonly log: (message: Omit<RuntimeLogMessage, 'type' | 'timestamp'>) => void;
    private readonly files = new Map<number, OpenResource>();
    private nextDescriptor = PREOPEN_FD + 1;
    private stderr = '';

    public constructor(
        memory: GuestMemory,
        log: (message: Omit<RuntimeLogMessage, 'type' | 'timestamp'>) => void,
    ) {
        this.memory = memory;
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
                this.memory.writeU32(countPointer, 0);
                this.memory.writeU32(sizePointer, 0);
                return ERRNO_SUCCESS;
            },
            environ_get: () => ERRNO_SUCCESS,
            clock_res_get: (clock: number, output: number) => {
                const resolution = clock === 0 ? 1_000_000n
                    : clock >= 1 && clock <= 3 ? 1n
                        : undefined;
                if (resolution === undefined) return ERRNO_INVAL;
                this.memory.writeU64(output, resolution);
                return ERRNO_SUCCESS;
            },
            clock_time_get: (clock: number, _precision: bigint, output: number) => {
                const time = this.clockTime(clock);
                if (time === undefined) return ERRNO_INVAL;
                this.memory.writeU64(output, time);
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
                if ((length >>> 0) < bytes.length) return ERRNO_NAMETOOLONG;
                this.memory.writeBytes(pointer, bytes);
                return ERRNO_SUCCESS;
            },
            fd_advise: (descriptor: number, _offset: bigint, _length: bigint, advice: number) => {
                return this.resource(descriptor) === undefined ? ERRNO_BADF
                    : advice >= 0 && advice <= 5 ? ERRNO_SUCCESS : ERRNO_INVAL;
            },
            fd_allocate: () => ERRNO_NOTCAPABLE,
            fd_close: (descriptor: number) => this.close(descriptor),
            fd_datasync: (descriptor: number) => this.knownDescriptor(descriptor)
                ? ERRNO_SUCCESS : ERRNO_BADF,
            fd_sync: (descriptor: number) => this.knownDescriptor(descriptor)
                ? ERRNO_SUCCESS : ERRNO_BADF,
            fd_fdstat_get: (descriptor: number, output: number) => this.fdstat(descriptor, output),
            fd_fdstat_set_flags: () => ERRNO_NOTCAPABLE,
            fd_fdstat_set_rights: (descriptor: number, base: bigint, inheriting: bigint) => {
                const resource = this.resource(descriptor);
                if (resource === undefined) return ERRNO_BADF;
                if ((base & ~resource.rightsBase) !== 0n
                    || (inheriting & ~resource.rightsInheriting) !== 0n) {
                    return ERRNO_NOTCAPABLE;
                }
                resource.rightsBase = base;
                resource.rightsInheriting = inheriting;
                return ERRNO_SUCCESS;
            },
            fd_filestat_get: (descriptor: number, output: number) => this.fdFilestat(descriptor, output),
            fd_filestat_set_size: () => ERRNO_NOTCAPABLE,
            fd_filestat_set_times: () => ERRNO_NOTCAPABLE,
            fd_pread: (
                descriptor: number,
                vectors: number,
                vectorCount: number,
                offset: bigint,
                output: number,
            ) => this.read(descriptor, vectors, vectorCount, output, offset),
            fd_pwrite: () => ERRNO_NOTCAPABLE,
            fd_read: (descriptor: number, vectors: number, vectorCount: number, output: number) => {
                const resource = this.resource(descriptor);
                if (descriptor === 0) {
                    this.memory.writeU32(output, 0);
                    return ERRNO_SUCCESS;
                }
                if (resource === undefined) return ERRNO_BADF;
                const result = this.read(descriptor, vectors, vectorCount, output, BigInt(resource.offset));
                if (result === ERRNO_SUCCESS) resource.offset += this.memory.readU32(output);
                return result;
            },
            fd_readdir: (
                descriptor: number,
                pointer: number,
                length: number,
                cookie: bigint,
                output: number,
            ) => this.readDirectory(descriptor, pointer, length, cookie, output),
            fd_renumber: (from: number, to: number) => this.renumber(from, to),
            fd_seek: (descriptor: number, offset: bigint, whence: number, output: number) => {
                const resource = this.resource(descriptor);
                if (resource === undefined) return ERRNO_BADF;
                if (resource.directory) return ERRNO_ISDIR;
                let base: bigint;
                if (whence === 0) base = 0n;
                else if (whence === 1) base = BigInt(resource.offset);
                else if (whence === 2 && resource.fd !== undefined) {
                    base = fstatSync(resource.fd, { bigint: true }).size;
                } else return ERRNO_INVAL;
                const position = base + offset;
                if (position < 0n || position > BigInt(Number.MAX_SAFE_INTEGER)) return ERRNO_INVAL;
                resource.offset = Number(position);
                this.memory.writeU64(output, position);
                return ERRNO_SUCCESS;
            },
            fd_tell: (descriptor: number, output: number) => {
                const resource = this.resource(descriptor);
                if (resource === undefined) return ERRNO_BADF;
                this.memory.writeU64(output, BigInt(resource.offset));
                return ERRNO_SUCCESS;
            },
            fd_write: (descriptor: number, vectors: number, vectorCount: number, output: number) => {
                if (descriptor !== 1 && descriptor !== 2) return ERRNO_NOTCAPABLE;
                let total = 0;
                for (let index = 0; index < (vectorCount >>> 0); index += 1) {
                    const pointer = this.memory.readU32(vectors + index * 8);
                    const length = this.memory.readU32(vectors + index * 8 + 4);
                    total += length;
                    if (descriptor === 2) {
                        this.stderr += decoder.decode(this.memory.readBytes(pointer, length), { stream: true });
                    }
                }
                this.memory.writeU32(output, total);
                this.flushStderr(false);
                return ERRNO_SUCCESS;
            },
            path_create_directory: () => ERRNO_NOTCAPABLE,
            path_filestat_get: (
                descriptor: number,
                lookupFlags: number,
                pathPointer: number,
                pathLength: number,
                output: number,
            ) => this.pathFilestat(descriptor, lookupFlags, pathPointer, pathLength, output),
            path_filestat_set_times: () => ERRNO_NOTCAPABLE,
            path_link: () => ERRNO_NOTCAPABLE,
            path_open: (
                descriptor: number,
                lookupFlags: number,
                pathPointer: number,
                pathLength: number,
                openFlags: number,
                rights: bigint,
                inheritingRights: bigint,
                descriptorFlags: number,
                output: number,
            ) => this.open(
                descriptor,
                lookupFlags,
                this.memory.readString(pathPointer, pathLength),
                openFlags,
                rights,
                inheritingRights,
                descriptorFlags,
                output,
            ),
            path_readlink: (
                descriptor: number,
                pathPointer: number,
                pathLength: number,
                buffer: number,
                bufferLength: number,
                output: number,
            ) => this.readLink(descriptor, pathPointer, pathLength, buffer, bufferLength, output),
            path_remove_directory: () => ERRNO_NOTCAPABLE,
            path_rename: () => ERRNO_NOTCAPABLE,
            path_symlink: () => ERRNO_NOTCAPABLE,
            path_unlink_file: () => ERRNO_NOTCAPABLE,
            poll_oneoff: (
                subscriptions: number,
                events: number,
                subscriptionCount: number,
                output: number,
            ) => this.poll(subscriptions, events, subscriptionCount, output),
            proc_exit: (code: number) => { throw new WasiExit(code >>> 0); },
            proc_raise: () => ERRNO_NOSYS,
            sched_yield: () => ERRNO_SUCCESS,
            sock_accept: () => ERRNO_NOSYS,
            sock_recv: () => ERRNO_NOSYS,
            sock_send: () => ERRNO_NOSYS,
            sock_shutdown: () => ERRNO_NOSYS,
        };
    }

    public dispose(): void {
        for (const descriptor of [...this.files.keys()]) this.close(descriptor);
        this.flushStderr(true);
    }

    private clockTime(clock: number): bigint | undefined {
        if (clock === 0) return BigInt(Date.now()) * 1_000_000n;
        if (clock === 1) return process.hrtime.bigint();
        if (clock === 2 || clock === 3) {
            const usage = process.cpuUsage();
            return BigInt(usage.user + usage.system) * 1_000n;
        }
        return undefined;
    }

    private knownDescriptor(descriptor: number): boolean {
        return descriptor >= 0 && descriptor <= PREOPEN_FD || this.files.has(descriptor);
    }

    private resource(descriptor: number): OpenResource | undefined {
        return this.files.get(descriptor);
    }

    private fdstat(descriptor: number, output: number): number {
        let fileType: number;
        let rightsBase = 0n;
        let rightsInheriting = 0n;
        if (descriptor >= 0 && descriptor <= 2) {
            fileType = FILETYPE_CHARACTER_DEVICE;
            if (descriptor === 0) rightsBase = RIGHT_FD_READ;
            else rightsBase = RIGHT_FD_WRITE;
        } else if (descriptor === PREOPEN_FD) {
            fileType = FILETYPE_DIRECTORY;
            rightsBase = PREOPEN_BASE_RIGHTS;
            rightsInheriting = PREOPEN_INHERITING_RIGHTS;
        } else {
            const resource = this.resource(descriptor);
            if (resource === undefined) return ERRNO_BADF;
            fileType = resource.directory ? FILETYPE_DIRECTORY : FILETYPE_REGULAR_FILE;
            rightsBase = resource.rightsBase;
            rightsInheriting = resource.rightsInheriting;
        }
        this.memory.writeBytes(output, new Uint8Array(24));
        this.memory.writeU8(output, fileType);
        this.memory.writeU64(output + 8, rightsBase);
        this.memory.writeU64(output + 16, rightsInheriting);
        return ERRNO_SUCCESS;
    }

    private fdFilestat(descriptor: number, output: number): number {
        if (descriptor >= 0 && descriptor <= 2) {
            this.memory.writeBytes(output, new Uint8Array(64));
            this.memory.writeU8(output + 16, FILETYPE_CHARACTER_DEVICE);
            return ERRNO_SUCCESS;
        }
        if (descriptor === PREOPEN_FD) {
            this.memory.writeBytes(output, new Uint8Array(64));
            this.memory.writeU8(output + 16, FILETYPE_DIRECTORY);
            this.memory.writeU64(output + 24, 1n);
            return ERRNO_SUCCESS;
        }
        const resource = this.resource(descriptor);
        if (resource === undefined) return ERRNO_BADF;
        try {
            return this.writeFilestat(output, statSync(resource.nativePath, { bigint: true }));
        } catch (error) {
            return errno(error);
        }
    }

    private pathFilestat(
        descriptor: number,
        lookupFlags: number,
        pathPointer: number,
        pathLength: number,
        output: number,
    ): number {
        try {
            const nativePath = this.resolve(descriptor, this.memory.readString(pathPointer, pathLength));
            const stat = (lookupFlags & 1) !== 0
                ? statSync(nativePath, { bigint: true })
                : lstatSync(nativePath, { bigint: true });
            return this.writeFilestat(output, stat);
        } catch (error) {
            return errno(error);
        }
    }

    private writeFilestat(output: number, stat: WasiStat): number {
        this.memory.writeBytes(output, new Uint8Array(64));
        this.memory.writeU64(output, stat.dev);
        this.memory.writeU64(output + 8, stat.ino);
        this.memory.writeU8(output + 16, stat.isFile()
            ? FILETYPE_REGULAR_FILE
            : stat.isDirectory()
                ? FILETYPE_DIRECTORY
                : stat.isSymbolicLink()
                    ? FILETYPE_SYMBOLIC_LINK
                    : FILETYPE_UNKNOWN);
        this.memory.writeU64(output + 24, stat.nlink);
        this.memory.writeU64(output + 32, stat.size);
        this.memory.writeU64(output + 40, stat.atimeNs);
        this.memory.writeU64(output + 48, stat.mtimeNs);
        this.memory.writeU64(output + 56, stat.ctimeNs);
        return ERRNO_SUCCESS;
    }

    private read(
        descriptor: number,
        vectors: number,
        vectorCount: number,
        output: number,
        initialOffset: bigint,
    ): number {
        const resource = this.resource(descriptor);
        if (resource === undefined || resource.fd === undefined) return ERRNO_BADF;
        if (initialOffset < 0n || initialOffset > BigInt(Number.MAX_SAFE_INTEGER)) return ERRNO_INVAL;
        let offset = Number(initialOffset);
        let total = 0;
        try {
            for (let index = 0; index < (vectorCount >>> 0); index += 1) {
                const pointer = this.memory.readU32(vectors + index * 8);
                const length = this.memory.readU32(vectors + index * 8 + 4);
                const bytes = this.memory.readBytes(pointer, length);
                const read = readSync(resource.fd, bytes, 0, length, offset);
                offset += read;
                total += read;
                if (read < length) break;
            }
            this.memory.writeU32(output, total);
            return ERRNO_SUCCESS;
        } catch (error) {
            return errno(error);
        }
    }

    private readDirectory(
        descriptor: number,
        pointer: number,
        length: number,
        cookie: bigint,
        output: number,
    ): number {
        const resource = this.resource(descriptor);
        if (resource === undefined) return ERRNO_BADF;
        if (!resource.directory) return ERRNO_NOTDIR;
        if (cookie < 0n || cookie > BigInt(Number.MAX_SAFE_INTEGER)) return ERRNO_INVAL;
        try {
            const entries = readdirSync(resource.nativePath, { withFileTypes: true });
            let used = 0;
            for (let index = Number(cookie); index < entries.length && used < (length >>> 0); index += 1) {
                const entry = entries[index];
                const name = encoder.encode(entry.name);
                const record = new Uint8Array(24 + name.length);
                const view = new DataView(record.buffer);
                view.setBigUint64(0, BigInt(index + 1), true);
                try {
                    const stat = lstatSync(path.join(resource.nativePath, entry.name), { bigint: true });
                    view.setBigUint64(8, stat.ino, true);
                } catch {}
                view.setUint32(16, name.length, true);
                record[20] = entry.isFile() ? FILETYPE_REGULAR_FILE
                    : entry.isDirectory() ? FILETYPE_DIRECTORY
                        : entry.isSymbolicLink() ? FILETYPE_SYMBOLIC_LINK : FILETYPE_UNKNOWN;
                record.set(name, 24);
                const writable = Math.min(record.length, (length >>> 0) - used);
                this.memory.writeBytes(pointer + used, record.subarray(0, writable));
                used += writable;
            }
            this.memory.writeU32(output, used);
            return ERRNO_SUCCESS;
        } catch (error) {
            return errno(error);
        }
    }

    private open(
        descriptor: number,
        lookupFlags: number,
        wasiPath: string,
        openFlags: number,
        rights: bigint,
        inheritingRights: bigint,
        descriptorFlags: number,
        output: number,
    ): number {
        const mutatingOpenFlags = 1 | 4 | 8;
        if ((rights & MUTATING_FILE_RIGHTS) !== 0n
            || (inheritingRights & MUTATING_FILE_RIGHTS) !== 0n
            || (openFlags & mutatingOpenFlags) !== 0
            || descriptorFlags !== 0) {
            return ERRNO_NOTCAPABLE;
        }
        try {
            const nativePath = this.resolve(descriptor, wasiPath);
            if ((lookupFlags & 1) === 0 && lstatSync(nativePath).isSymbolicLink()) return ERRNO_LOOP;
            const stat = statSync(nativePath);
            const wantsDirectory = (openFlags & 2) !== 0;
            if (wantsDirectory && !stat.isDirectory()) return ERRNO_NOTDIR;
            const opened = this.nextDescriptor++;
            const directory = stat.isDirectory();
            this.files.set(opened, {
                nativePath,
                fd: directory ? undefined : openSync(nativePath, 'r'),
                offset: 0,
                directory,
                rightsBase: rights,
                rightsInheriting: inheritingRights,
            });
            this.memory.writeU32(output, opened);
            return ERRNO_SUCCESS;
        } catch (error) {
            return errno(error);
        }
    }

    private readLink(
        descriptor: number,
        pathPointer: number,
        pathLength: number,
        buffer: number,
        bufferLength: number,
        output: number,
    ): number {
        try {
            const nativePath = this.resolve(descriptor, this.memory.readString(pathPointer, pathLength));
            const bytes = encoder.encode(readlinkSync(nativePath));
            const written = Math.min(bytes.length, bufferLength >>> 0);
            this.memory.writeBytes(buffer, bytes.subarray(0, written));
            this.memory.writeU32(output, written);
            return ERRNO_SUCCESS;
        } catch (error) {
            return errno(error);
        }
    }

    private poll(subscriptions: number, events: number, count: number, output: number): number {
        const parsed: Array<{ userdata: bigint; type: number; deadline?: bigint; descriptor?: number }> = [];
        let hasImmediatelyReady = false;
        let firstDeadline: bigint | undefined;
        for (let index = 0; index < (count >>> 0); index += 1) {
            const pointer = subscriptions + index * 48;
            const userdata = this.memory.readU64(pointer);
            const type = this.memory.readU8(pointer + 8);
            if (type === 0) {
                const clock = this.memory.readU32(pointer + 16);
                const now = this.clockTime(clock);
                if (now === undefined) return ERRNO_INVAL;
                const timeout = this.memory.readU64(pointer + 24);
                const flags = this.memory.readU16(pointer + 40);
                const monotonicNow = process.hrtime.bigint();
                const delay = (flags & 1) !== 0
                    ? timeout > now ? timeout - now : 0n
                    : timeout;
                const deadline = monotonicNow + delay;
                parsed.push({ userdata, type, deadline });
                if (firstDeadline === undefined || deadline < firstDeadline) firstDeadline = deadline;
            } else if (type === 1 || type === 2) {
                parsed.push({ userdata, type, descriptor: this.memory.readU32(pointer + 16) });
                hasImmediatelyReady = true;
            } else {
                return ERRNO_INVAL;
            }
        }
        if (!hasImmediatelyReady && firstDeadline !== undefined) {
            const delay = firstDeadline - process.hrtime.bigint();
            if (delay > 0n) {
                Atomics.wait(
                    new Int32Array(new SharedArrayBuffer(4)),
                    0,
                    0,
                    Number(delay) / 1_000_000,
                );
            }
        }
        const now = process.hrtime.bigint();
        let written = 0;
        for (const subscription of parsed) {
            if (subscription.deadline !== undefined && subscription.deadline > now && hasImmediatelyReady) continue;
            const pointer = events + written * 32;
            this.memory.writeBytes(pointer, new Uint8Array(32));
            this.memory.writeU64(pointer, subscription.userdata);
            this.memory.writeU8(pointer + 10, subscription.type);
            if (subscription.descriptor !== undefined && !this.knownDescriptor(subscription.descriptor)) {
                this.memory.writeU16(pointer + 8, ERRNO_BADF);
            }
            written += 1;
        }
        this.memory.writeU32(output, written);
        return ERRNO_SUCCESS;
    }

    private renumber(from: number, to: number): number {
        const resource = this.files.get(from);
        if (resource === undefined || to <= PREOPEN_FD) return ERRNO_BADF;
        this.close(to);
        this.files.delete(from);
        this.files.set(to, resource);
        return ERRNO_SUCCESS;
    }

    private resolve(descriptor: number, wasiPath: string): string {
        if (descriptor === PREOPEN_FD) return wasiRelativePathToNative(wasiPath);
        const resource = this.resource(descriptor);
        if (resource === undefined) throw nodeError('EBADF');
        if (!resource.directory) throw nodeError('ENOTDIR');
        return path.join(resource.nativePath, ...relativeSegments(wasiPath));
    }

    private close(descriptor: number): number {
        const resource = this.files.get(descriptor);
        if (resource === undefined) return ERRNO_BADF;
        this.files.delete(descriptor);
        if (resource.fd === undefined) return ERRNO_SUCCESS;
        try {
            closeSync(resource.fd);
            return ERRNO_SUCCESS;
        } catch (error) {
            return errno(error);
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
    const resolved = path.resolve(nativePath.replace(/^\\\\\?\\/, ''));
    if (process.platform === 'win32') {
        const match = /^([a-zA-Z]):[\\/](.*)$/.exec(resolved);
        if (match === null) throw new Error(`Cannot expose ${nativePath} through /mnt.`);
        return `/mnt/${match[1].toLowerCase()}/${match[2].replaceAll('\\', '/')}`;
    }
    return `/mnt${resolved}`;
}

export function wasiRelativePathToNative(wasiPath: string): string {
    const segments = relativeSegments(wasiPath);
    if (process.platform === 'win32') {
        const [drive, ...rest] = segments;
        if (drive === undefined || !/^[a-zA-Z]$/.test(drive)) {
            throw new WasiPathError('A Windows WASI path must begin with a drive letter.');
        }
        return path.win32.join(`${drive}:\\`, ...rest);
    }
    return path.posix.join('/', ...segments);
}

function relativeSegments(wasiPath: string): string[] {
    if (wasiPath.startsWith('/') || wasiPath.includes('\\')) {
        throw new WasiPathError('WASI preopen paths must be relative and use forward slashes.');
    }
    const segments = wasiPath.split('/').filter(segment => segment.length > 0);
    if (segments.some(segment => segment === '.' || segment === '..' || segment.includes('\0'))) {
        throw new WasiPathError('WASI path escapes the read-only preopen.');
    }
    return segments;
}

function errno(error: unknown): number {
    if (error instanceof WasiPathError) return ERRNO_NOTCAPABLE;
    const code = (error as NodeJS.ErrnoException).code;
    switch (code) {
        case 'EACCES': case 'EPERM': return ERRNO_ACCES;
        case 'EBADF': return ERRNO_BADF;
        case 'EEXIST': return ERRNO_EXIST;
        case 'EISDIR': return ERRNO_ISDIR;
        case 'ELOOP': return ERRNO_LOOP;
        case 'ENAMETOOLONG': return ERRNO_NAMETOOLONG;
        case 'ENOENT': return ERRNO_NOENT;
        case 'ENOTDIR': return ERRNO_NOTDIR;
        case 'ENOTEMPTY': return ERRNO_NOTEMPTY;
        default: return ERRNO_IO;
    }
}

function nodeError(code: string): NodeJS.ErrnoException {
    return Object.assign(new Error(code), { code });
}
