import { createRequire } from 'node:module';

import type { ProcessSnapshot, RuntimeLogMessage } from '../runtimeProtocol';
import { GuestMemory } from './memory.ts';
import { nativePathToWasi } from './wasi.ts';

export interface NativeProcessBridge {
    listProcessesByName(name: string): number[];
    attachByName(name: string): number;
    attachByPid(pid: number): number;
    detach(handle: number): boolean;
    processId(handle: number): number;
    processPath(handle: number): string | null;
    isOpen(handle: number): boolean;
    readProcessMemory(handle: number, address: string, length: number): Uint8Array;
    moduleAddress(handle: number, module: string): string | null;
    moduleSize(handle: number, module: string): string | null;
    modulePath(handle: number, module: string): string | null;
    memoryRangeCount(handle: number): number;
    memoryRangeAddress(handle: number, index: number): string | null;
    memoryRangeSize(handle: number, index: number): string | null;
    memoryRangeFlags(handle: number, index: number): string | null;
}

interface AttachedProcess {
    nativeHandle: number;
    pid: number;
    path?: string;
    isOpen: boolean;
}

export class ProcessHost {
    private readonly memory: GuestMemory;
    private readonly bridge: NativeProcessBridge;
    private readonly log: (message: Omit<RuntimeLogMessage, 'type' | 'timestamp'>) => void;
    private readonly processes = new Map<bigint, AttachedProcess>();
    private nextHandle = 1n;
    private changed = false;

    public constructor(
        memory: GuestMemory,
        bridge: NativeProcessBridge,
        log: (message: Omit<RuntimeLogMessage, 'type' | 'timestamp'>) => void,
    ) {
        this.memory = memory;
        this.bridge = bridge;
        this.log = log;
    }

    public imports(): Record<string, WebAssembly.ImportValue> {
        return {
            process_attach: (pointer: number, length: number) => {
                const name = this.memory.readString(pointer, length);
                try {
                    return this.attach(this.bridge.attachByName(name));
                } catch {
                    return 0n;
                }
            },
            process_attach_by_pid: (pid: bigint) => {
                if (pid < 0n || pid > 0xffff_ffffn) return 0n;
                try {
                    return this.attach(this.bridge.attachByPid(Number(pid)));
                } catch {
                    return 0n;
                }
            },
            process_list_by_name: (
                namePointer: number,
                nameLength: number,
                listPointer: number,
                listLengthPointer: number,
            ) => {
                const name = this.memory.readString(namePointer, nameLength);
                const capacity = this.memory.readU32(listLengthPointer);
                const byteLength = capacity * 8;
                if (!Number.isSafeInteger(byteLength)) {
                    throw new WebAssembly.RuntimeError('the process list capacity overflows guest memory');
                }
                this.memory.readBytes(listPointer, byteLength);
                try {
                    const pids = this.bridge.listProcessesByName(name);
                    for (let index = 0; index < Math.min(capacity, pids.length); index += 1) {
                        this.memory.writeU64(listPointer + index * 8, BigInt(pids[index]));
                    }
                    this.memory.writeU32(listLengthPointer, pids.length);
                    return 1;
                } catch {
                    return 0;
                }
            },
            process_detach: (handle: bigint) => {
                const process = this.process(handle);
                if (!this.bridge.detach(process.nativeHandle)) {
                    throw new WebAssembly.RuntimeError(`Invalid native process handle: ${process.nativeHandle}`);
                }
                this.processes.delete(handle);
                this.changed = true;
                this.log({ source: 'runtime', level: 'debug', message: `Detached from PID ${process.pid}.` });
            },
            process_is_open: (handle: bigint) => {
                const process = this.process(handle);
                const isOpen = this.bridge.isOpen(process.nativeHandle);
                if (isOpen !== process.isOpen) {
                    process.isOpen = isOpen;
                    this.changed = true;
                }
                return Number(process.isOpen);
            },
            process_read: (
                handle: bigint,
                address: bigint,
                pointer: number,
                length: number,
            ) => {
                const process = this.process(handle);
                this.memory.readBytes(pointer, length);
                try {
                    const bytes = this.bridge.readProcessMemory(
                        process.nativeHandle,
                        BigInt.asUintN(64, address).toString(),
                        length,
                    );
                    if (bytes.length !== length) return 0;
                    this.memory.writeBytes(pointer, bytes);
                    return 1;
                } catch {
                    return 0;
                }
            },
            process_get_module_address: (handle: bigint, pointer: number, length: number) => {
                return this.moduleNumber(handle, pointer, length, 'moduleAddress');
            },
            process_get_module_size: (handle: bigint, pointer: number, length: number) => {
                return this.moduleNumber(handle, pointer, length, 'moduleSize');
            },
            process_get_module_path: (
                handle: bigint,
                namePointer: number,
                nameLength: number,
                pathPointer: number,
                pathLengthPointer: number,
            ) => {
                const process = this.process(handle);
                const name = this.memory.readString(namePointer, nameLength);
                try {
                    const nativePath = this.bridge.modulePath(process.nativeHandle, name);
                    return nativePath === null
                        ? this.writeMissingString(pathLengthPointer)
                        : this.memory.writeHostString(
                            pathPointer,
                            pathLengthPointer,
                            nativePathToWasi(nativePath),
                        );
                } catch {
                    return this.writeMissingString(pathLengthPointer);
                }
            },
            process_get_path: (handle: bigint, pointer: number, lengthPointer: number) => {
                const process = this.process(handle);
                if (process.path === undefined) return this.writeMissingString(lengthPointer);
                return this.memory.writeHostString(pointer, lengthPointer, process.path);
            },
            process_get_memory_range_count: (handle: bigint) => {
                const process = this.process(handle);
                try {
                    return BigInt(this.bridge.memoryRangeCount(process.nativeHandle));
                } catch {
                    return 0n;
                }
            },
            process_get_memory_range_address: (handle: bigint, index: bigint) => {
                return this.memoryRangeNumber(handle, index, 'memoryRangeAddress');
            },
            process_get_memory_range_size: (handle: bigint, index: bigint) => {
                return this.memoryRangeNumber(handle, index, 'memoryRangeSize');
            },
            process_get_memory_range_flags: (handle: bigint, index: bigint) => {
                return this.memoryRangeNumber(handle, index, 'memoryRangeFlags');
            },
        };
    }

    public snapshot(): ProcessSnapshot[] {
        return [...this.processes].map(([handle, process]) => ({
            handle: handle.toString(),
            pid: process.pid,
            path: process.path,
            isOpen: process.isOpen,
        }));
    }

    public consumeChanged(): boolean {
        const changed = this.changed;
        this.changed = false;
        return changed;
    }

    public dispose(): void {
        for (const process of this.processes.values()) {
            try {
                this.bridge.detach(process.nativeHandle);
            } catch {
                // The worker is already shutting down; continue releasing the rest.
            }
        }
        this.processes.clear();
        this.changed = true;
    }

    private attach(nativeHandle: number): bigint {
        let nativePath: string | null;
        let pid: number;
        try {
            nativePath = this.bridge.processPath(nativeHandle);
            pid = this.bridge.processId(nativeHandle);
        } catch (error) {
            try {
                this.bridge.detach(nativeHandle);
            } catch {
                // Preserve the original error from querying the new handle.
            }
            throw error;
        }
        const handle = this.nextHandle++;
        let portablePath: string | undefined;
        if (nativePath !== null) {
            try {
                portablePath = nativePathToWasi(nativePath);
            } catch {
                portablePath = nativePath;
            }
        }
        this.processes.set(handle, { nativeHandle, pid, path: portablePath, isOpen: true });
        this.changed = true;
        this.log({ source: 'runtime', level: 'debug', message: `Attached to PID ${pid}.` });
        return handle;
    }

    private process(handle: bigint): AttachedProcess {
        const process = this.processes.get(handle);
        if (process === undefined) {
            throw new WebAssembly.RuntimeError(`Invalid process handle: ${handle}`);
        }
        return process;
    }

    private moduleNumber(
        handle: bigint,
        pointer: number,
        length: number,
        operation: 'moduleAddress' | 'moduleSize',
    ): bigint {
        const process = this.process(handle);
        const module = this.memory.readString(pointer, length);
        try {
            return parseNativeU64(this.bridge[operation](process.nativeHandle, module));
        } catch {
            return 0n;
        }
    }

    private memoryRangeNumber(
        handle: bigint,
        index: bigint,
        operation: 'memoryRangeAddress' | 'memoryRangeSize' | 'memoryRangeFlags',
    ): bigint {
        const process = this.process(handle);
        if (index < 0n || index > 0xffff_ffffn) return 0n;
        try {
            return parseNativeU64(this.bridge[operation](process.nativeHandle, Number(index)));
        } catch {
            return 0n;
        }
    }

    private writeMissingString(lengthPointer: number): number {
        this.memory.writeU32(lengthPointer, 0);
        return 0;
    }
}

export function loadNativeProcessBridge(modulePath: string): NativeProcessBridge {
    const require = createRequire(__filename);
    return require(modulePath) as NativeProcessBridge;
}

function parseNativeU64(value: string | null): bigint {
    return value === null ? 0n : BigInt(value);
}
