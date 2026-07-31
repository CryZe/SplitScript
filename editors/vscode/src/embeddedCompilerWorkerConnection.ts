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
        const response = await this.request({ kind: 'compile', request });
        if (response === undefined) {
            throw new Error('embedded compiler returned no compile response');
        }
        return response;
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
        return new Promise((resolve, reject) => {
            this.pending.set(id, { resolve, reject });
            this.port.postMessage({ ...request, id }, transfer);
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
            const error = new Error(message.error.message);
            error.name = message.error.name;
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
