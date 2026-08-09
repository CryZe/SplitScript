const RESPONSE_MAGIC = new Uint8Array([0x53, 0x53, 0x43, 0x52]);

export const compilerServiceProtocolVersion = 1;

export type EmbeddedBuildProfile = 'debug' | 'release';
export type EmbeddedWarningLevel = 'allow' | 'warn' | 'deny';

export interface EmbeddedWarningPolicy {
    mustUse?: EmbeddedWarningLevel;
    unusedBinding?: EmbeddedWarningLevel;
    unusedDeclaration?: EmbeddedWarningLevel;
    unusedMember?: EmbeddedWarningLevel;
}

export interface EmbeddedCompileRequest {
    protocolVersion: number;
    uri: string;
    revision: number;
    source: string;
    profile: EmbeddedBuildProfile;
    /** Optional host/project policy. Omission preserves the default warn level. */
    warnings?: EmbeddedWarningPolicy;
}

export interface EmbeddedCompilerIdentity {
    version: string;
    gitRevision: string | null;
}

export interface EmbeddedServiceSpan {
    start: number;
    end: number;
}

export interface EmbeddedServiceDiagnostic {
    code: string;
    severity: 'error' | 'warning' | 'information' | 'hint';
    message: string;
    span: EmbeddedServiceSpan;
    labels: readonly unknown[];
    notes: readonly string[];
    fixes: readonly unknown[];
}

interface EmbeddedServiceError {
    code: 'unsupportedProtocol' | 'sourceTooLarge' | 'invalidRequest' | 'internal';
    message: string;
}

interface EmbeddedCompileMetadata {
    protocolVersion: number;
    compiler: EmbeddedCompilerIdentity;
    uri: string;
    revision: number;
    diagnostics: EmbeddedServiceDiagnostic[];
    artifactLength: number;
    error: EmbeddedServiceError | null;
}

export interface EmbeddedCompileResponse {
    protocolVersion: number;
    compiler: EmbeddedCompilerIdentity;
    uri: string;
    revision: number;
    diagnostics: readonly EmbeddedServiceDiagnostic[];
    artifact: Uint8Array | undefined;
}

interface EmbeddedCompilerExports extends WebAssembly.Exports {
    memory: WebAssembly.Memory;
    splitscript_service_protocol_version(): number;
    splitscript_service_alloc(length: number): number;
    splitscript_service_dealloc(pointer: number, length: number): void;
    splitscript_service_compile(pointer: number, length: number): void;
    splitscript_service_response_pointer(): number;
    splitscript_service_response_length(): number;
}

export class EmbeddedCompilerProtocolError extends Error {
    public readonly code: string;

    public constructor(
        code: string,
        message: string,
    ) {
        super(message);
        this.name = 'EmbeddedCompilerProtocolError';
        this.code = code;
    }
}

/**
 * In-process binding for the direct WebAssembly compiler prototype.
 *
 * Production extension integration will own this object in a worker. Keeping
 * the binding free of Node and VS Code APIs makes that move mechanical.
 */
export class EmbeddedCompiler {
    private readonly encoder = new TextEncoder();
    private readonly exports: EmbeddedCompilerExports;

    private constructor(exports: EmbeddedCompilerExports) {
        this.exports = exports;
    }

    public static async create(moduleBytes: Uint8Array): Promise<EmbeddedCompiler> {
        const ownedModuleBytes = new Uint8Array(moduleBytes.length);
        ownedModuleBytes.set(moduleBytes);
        const module = await WebAssembly.compile(ownedModuleBytes.buffer);
        const instance = await WebAssembly.instantiate(module, {});
        const wasmExports = instance.exports as unknown as EmbeddedCompilerExports;
        const version = wasmExports.splitscript_service_protocol_version();
        if (version !== compilerServiceProtocolVersion) {
            throw new EmbeddedCompilerProtocolError(
                'unsupportedProtocol',
                `embedded compiler protocol ${version} does not match extension protocol ${compilerServiceProtocolVersion}`,
            );
        }
        return new EmbeddedCompiler(wasmExports);
    }

    public compile(request: EmbeddedCompileRequest): EmbeddedCompileResponse {
        const requestBytes = this.encoder.encode(JSON.stringify(request));
        const pointer = this.exports.splitscript_service_alloc(requestBytes.length);
        try {
            new Uint8Array(this.exports.memory.buffer, pointer, requestBytes.length)
                .set(requestBytes);
            this.exports.splitscript_service_compile(pointer, requestBytes.length);
        } finally {
            this.exports.splitscript_service_dealloc(pointer, requestBytes.length);
        }

        const responsePointer = this.exports.splitscript_service_response_pointer();
        const responseLength = this.exports.splitscript_service_response_length();
        const response = new Uint8Array(
            this.exports.memory.buffer,
            responsePointer,
            responseLength,
        ).slice();
        return decodeEmbeddedCompileResponse(response);
    }
}

export function decodeEmbeddedCompileResponse(
    response: Uint8Array,
): EmbeddedCompileResponse {
    if (
        response.length < 8
        || !RESPONSE_MAGIC.every((byte, index) => response[index] === byte)
    ) {
        throw new EmbeddedCompilerProtocolError(
            'invalidResponse',
            'embedded compiler returned an invalid response envelope',
        );
    }
    const metadataLength = new DataView(
        response.buffer,
        response.byteOffset + 4,
        4,
    ).getUint32(0, true);
    const artifactOffset = 8 + metadataLength;
    if (artifactOffset > response.length) {
        throw new EmbeddedCompilerProtocolError(
            'invalidResponse',
            'embedded compiler response metadata extends past the envelope',
        );
    }
    let metadata: EmbeddedCompileMetadata;
    try {
        metadata = JSON.parse(
            new TextDecoder().decode(response.subarray(8, artifactOffset)),
        ) as EmbeddedCompileMetadata;
    } catch (error) {
        throw new EmbeddedCompilerProtocolError(
            'invalidResponse',
            `embedded compiler returned invalid metadata: ${String(error)}`,
        );
    }
    if (metadata.error !== null) {
        throw new EmbeddedCompilerProtocolError(metadata.error.code, metadata.error.message);
    }
    const artifact = response.subarray(artifactOffset);
    if (artifact.length !== metadata.artifactLength) {
        throw new EmbeddedCompilerProtocolError(
            'invalidResponse',
            `embedded compiler declared ${metadata.artifactLength} artifact bytes but returned ${artifact.length}`,
        );
    }
    return {
        protocolVersion: metadata.protocolVersion,
        compiler: metadata.compiler,
        uri: metadata.uri,
        revision: metadata.revision,
        diagnostics: metadata.diagnostics,
        artifact: artifact.length === 0 ? undefined : artifact.slice(),
    };
}
