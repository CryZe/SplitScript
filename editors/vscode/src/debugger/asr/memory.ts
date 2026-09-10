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

    public copy(): Uint8Array<ArrayBuffer> {
        const memory = this.memory;
        if (memory === undefined) {
            throw new WebAssembly.RuntimeError('guest memory is not available yet');
        }
        const copy = new Uint8Array(new ArrayBuffer(memory.buffer.byteLength));
        copy.set(new Uint8Array(memory.buffer));
        return copy;
    }

    public readString(pointer: number, length: number): string {
        return this.decoder.decode(this.slice(pointer, length));
    }

    public readBytes(pointer: number, length: number): Uint8Array {
        return this.slice(pointer, length);
    }

    public writeBytes(pointer: number, bytes: Uint8Array): void {
        this.slice(pointer, bytes.length).set(bytes);
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

    public readU32(pointer: number): number {
        const bytes = this.slice(pointer, 4);
        return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(0, true);
    }

    public writeU8(pointer: number, value: number): void {
        this.slice(pointer, 1)[0] = value;
    }

    public writeU32(pointer: number, value: number): void {
        const bytes = this.slice(pointer, 4);
        new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).setUint32(0, value, true);
    }

    public writeU64(pointer: number, value: bigint): void {
        const bytes = this.slice(pointer, 8);
        new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).setBigUint64(0, value, true);
    }

    public writeI64(pointer: number, value: bigint): void {
        const bytes = this.slice(pointer, 8);
        new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).setBigInt64(0, value, true);
    }

    public writeF64(pointer: number, value: number): void {
        const bytes = this.slice(pointer, 8);
        new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).setFloat64(0, value, true);
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
