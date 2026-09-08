export class GuestMemory {
    private memory: WebAssembly.Memory | undefined;
    private readonly decoder = new TextDecoder('utf-8', { fatal: true });
    private readonly encoder = new TextEncoder();

    public bind(memory: WebAssembly.Memory): void {
        this.memory = memory;
    }

    public byteLength(): number {
        return this.memory?.buffer.byteLength ?? 0;
    }

    public readString(pointer: number, length: number): string {
        return this.decoder.decode(this.slice(pointer, length));
    }

    public writeHostString(pointer: number, lengthPointer: number, value: string): number {
        const bytes = this.encoder.encode(value);
        const length = this.readU32(lengthPointer);
        this.writeU32(lengthPointer, bytes.length);
        if (length < bytes.length) {
            return 0;
        }
        this.slice(pointer, bytes.length).set(bytes);
        return 1;
    }

    private readU32(pointer: number): number {
        const bytes = this.slice(pointer, 4);
        return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(0, true);
    }

    private writeU32(pointer: number, value: number): void {
        const bytes = this.slice(pointer, 4);
        new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).setUint32(0, value, true);
    }

    private slice(pointer: number, length: number): Uint8Array {
        if (!Number.isInteger(pointer) || pointer < 0 || !Number.isInteger(length) || length < 0) {
            throw new WebAssembly.RuntimeError('guest pointer and length must be unsigned integers');
        }
        const memory = this.memory;
        if (memory === undefined) {
            throw new WebAssembly.RuntimeError('guest memory is not available yet');
        }
        const end = pointer + length;
        if (!Number.isSafeInteger(end) || end > memory.buffer.byteLength) {
            throw new WebAssembly.RuntimeError(
                `guest memory access ${pointer}..${end} exceeds ${memory.buffer.byteLength} bytes`,
            );
        }
        return new Uint8Array(memory.buffer, pointer, length);
    }
}
