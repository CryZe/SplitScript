import type { Message } from 'vscode-jsonrpc';

export const languageServerProtocolVersion = 1;

interface EmbeddedLanguageServerExports extends WebAssembly.Exports {
    memory: WebAssembly.Memory;
    splitscript_service_alloc(length: number): number;
    splitscript_service_dealloc(pointer: number, length: number): void;
    splitscript_lsp_protocol_version(): number;
    splitscript_lsp_handle(pointer: number, length: number): void;
    splitscript_lsp_response_pointer(): number;
    splitscript_lsp_response_length(): number;
}

/** A stateful LSP endpoint backed by one isolated compiler Wasm instance. */
export class EmbeddedLanguageServer {
    private readonly encoder = new TextEncoder();
    private readonly decoder = new TextDecoder();

    private constructor(private readonly exports: EmbeddedLanguageServerExports) {}

    public static async create(moduleBytes: Uint8Array): Promise<EmbeddedLanguageServer> {
        const ownedModuleBytes = new Uint8Array(moduleBytes.length);
        ownedModuleBytes.set(moduleBytes);
        const module = await WebAssembly.compile(ownedModuleBytes.buffer);
        const instance = await WebAssembly.instantiate(module, {});
        const wasmExports = instance.exports as unknown as EmbeddedLanguageServerExports;
        const version = wasmExports.splitscript_lsp_protocol_version();
        if (version !== languageServerProtocolVersion) {
            throw new Error(
                `embedded language-server protocol ${version} does not match extension protocol ${languageServerProtocolVersion}`,
            );
        }
        return new EmbeddedLanguageServer(wasmExports);
    }

    public handle(message: Message): readonly Message[] {
        const request = this.encoder.encode(JSON.stringify(message));
        const pointer = this.exports.splitscript_service_alloc(request.length);
        try {
            new Uint8Array(this.exports.memory.buffer, pointer, request.length).set(request);
            this.exports.splitscript_lsp_handle(pointer, request.length);
        } finally {
            this.exports.splitscript_service_dealloc(pointer, request.length);
        }

        const responsePointer = this.exports.splitscript_lsp_response_pointer();
        const responseLength = this.exports.splitscript_lsp_response_length();
        const response = new Uint8Array(
            this.exports.memory.buffer,
            responsePointer,
            responseLength,
        );
        const outgoing: unknown = JSON.parse(this.decoder.decode(response));
        if (!Array.isArray(outgoing)) {
            throw new Error('embedded language server returned a non-array response');
        }
        return outgoing as Message[];
    }
}
