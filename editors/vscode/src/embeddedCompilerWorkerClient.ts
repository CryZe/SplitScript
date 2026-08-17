import { Worker } from 'node:worker_threads';
import type {
    EmbeddedCompileRequest,
    EmbeddedCompileResponse,
} from './embeddedCompiler';
import {
    EmbeddedCompilerWorkerConnection,
    type EmbeddedCompilerWorkerPort,
} from './embeddedCompilerWorkerConnection';

export class EmbeddedCompilerWorkerClient {
    private constructor(private readonly connection: EmbeddedCompilerWorkerConnection) {}

    public static async create(
        workerPath: string,
        moduleBytes: Uint8Array,
    ): Promise<EmbeddedCompilerWorkerClient> {
        const worker = new Worker(workerPath);
        const port: EmbeddedCompilerWorkerPort = {
            onMessage: listener => worker.on('message', listener),
            onFailure: listener => {
                worker.on('error', listener);
                worker.on('exit', code => {
                    listener(new Error(`embedded compiler worker exited with code ${code}`));
                });
            },
            postMessage: (message, transfer) => worker.postMessage(message, [...transfer]),
            terminate: () => {
                void worker.terminate();
            },
        };
        return new EmbeddedCompilerWorkerClient(
            await EmbeddedCompilerWorkerConnection.create(port, moduleBytes),
        );
    }

    public compile(request: EmbeddedCompileRequest): Promise<EmbeddedCompileResponse> {
        return this.connection.compile(request);
    }

    public cancelCurrentCompilation(): void {
        this.connection.cancelCurrentCompilation();
    }

    public dispose(): void {
        this.connection.dispose();
    }
}
