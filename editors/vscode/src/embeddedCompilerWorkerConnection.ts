import type {
    EmbeddedCompileRequest,
    EmbeddedCompileResponse,
} from './embeddedCompiler';
import type {
    EmbeddedCompilerWorkerRequest,
    EmbeddedCompilerWorkerResponse,
} from './embeddedCompilerWorkerProtocol';

interface PendingRequest {
    resolve(value: EmbeddedCompileResponse | undefined): void;
    reject(error: Error): void;
}

type WithoutId<T> = T extends unknown ? Omit<T, 'id'> : never;
type EmbeddedCompilerWorkerRequestInput = WithoutId<EmbeddedCompilerWorkerRequest>;

export interface EmbeddedCompilerWorkerPort {
    onMessage(listener: (message: EmbeddedCompilerWorkerResponse) => void): void;
    onFailure(listener: (error: Error) => void): void;
    postMessage(
        message: EmbeddedCompilerWorkerRequest,
        transfer: readonly ArrayBuffer[],
    ): void;
    terminate(): void;
}

/** Shared request ownership and failure handling for every worker host. */
export class EmbeddedCompilerWorkerConnection {
    private readonly pending = new Map<number, PendingRequest>();
    private readonly port: EmbeddedCompilerWorkerPort;
    private nextRequestId = 1;
    private stopped = false;
    private activeCompileRequestId: number | undefined;
    private cancellationRequested = false;

    private constructor(port: EmbeddedCompilerWorkerPort) {
        this.port = port;
        port.onMessage(message => this.handleResponse(message));
        port.onFailure(error => this.stop(error));
    }

    public static async create(
        port: EmbeddedCompilerWorkerPort,
        moduleBytes: Uint8Array,
    ): Promise<EmbeddedCompilerWorkerConnection> {
        const connection = new EmbeddedCompilerWorkerConnection(port);
        const ownedModuleBytes = new Uint8Array(moduleBytes.length);
        ownedModuleBytes.set(moduleBytes);
        try {
            await connection.request({
                kind: 'initialize',
                moduleBytes: ownedModuleBytes.buffer,
            }, [ownedModuleBytes.buffer]);
        } catch (error) {
            connection.dispose();
            throw error;
        }
        return connection;
    }

    public async compile(request: EmbeddedCompileRequest): Promise<EmbeddedCompileResponse> {
        const id = this.nextRequestId++;
        this.activeCompileRequestId = id;
        this.cancellationRequested = false;
        try {
            const response = await this.requestWithId({ id, kind: 'compile', request });
            if (response === undefined) {
                throw new Error('embedded compiler returned no compile response');
            }
            return response;
        } finally {
            if (this.activeCompileRequestId === id) {
                this.activeCompileRequestId = undefined;
                this.cancellationRequested = false;
            }
        }
    }

    /** Requests that the worker discard the currently retained compiler stage. */
    public cancelCurrentCompilation(): void {
        const targetId = this.activeCompileRequestId;
        if (targetId === undefined || this.cancellationRequested || this.stopped) {
            return;
        }
        this.cancellationRequested = true;
        void this.request({ kind: 'cancel', targetId }).catch(() => {
            // The compile request owns user-visible failure reporting. A worker
            // failure rejects it as well, so the cancellation acknowledgement
            // adds no independent error surface.
        });
    }

    public dispose(): void {
        this.stop(new Error('embedded compiler worker was disposed'));
        this.port.terminate();
    }

    private request(
        request: EmbeddedCompilerWorkerRequestInput,
        transfer: readonly ArrayBuffer[] = [],
    ): Promise<EmbeddedCompileResponse | undefined> {
        if (this.stopped) {
            return Promise.reject(new Error('embedded compiler worker is stopped'));
        }
        const id = this.nextRequestId++;
        return this.requestWithId({ ...request, id }, transfer);
    }

    private requestWithId(
        request: EmbeddedCompilerWorkerRequest,
        transfer: readonly ArrayBuffer[] = [],
    ): Promise<EmbeddedCompileResponse | undefined> {
        const id = request.id;
        return new Promise((resolve, reject) => {
            this.pending.set(id, { resolve, reject });
            this.port.postMessage(request, transfer);
        });
    }

    private handleResponse(message: EmbeddedCompilerWorkerResponse): void {
        const pending = this.pending.get(message.id);
        if (pending === undefined) {
            return;
        }
        this.pending.delete(message.id);
        if (message.ok) {
            pending.resolve(message.response);
        } else {
            const error = new Error(message.error.message) as Error & { code?: string };
            error.name = message.error.name;
            if (message.error.code !== undefined) {
                error.code = message.error.code;
            }
            pending.reject(error);
        }
    }

    private stop(error: Error): void {
        if (this.stopped) {
            return;
        }
        this.stopped = true;
        for (const pending of this.pending.values()) {
            pending.reject(error);
        }
        this.pending.clear();
    }
}
