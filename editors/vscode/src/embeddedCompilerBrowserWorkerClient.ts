import type {
    EmbeddedCompileRequest,
    EmbeddedCompileResponse,
} from './embeddedCompiler';
import {
    EmbeddedCompilerWorkerConnection,
    type EmbeddedCompilerWorkerPort,
} from './embeddedCompilerWorkerConnection';

export class EmbeddedCompilerBrowserWorkerClient {
    private constructor(private readonly connection: EmbeddedCompilerWorkerConnection) {}

    public static async create(
        workerUrl: string,
        moduleBytes: Uint8Array,
    ): Promise<EmbeddedCompilerBrowserWorkerClient> {
        const worker = new Worker(workerUrl);
        const port: EmbeddedCompilerWorkerPort = {
            onMessage: listener => worker.addEventListener(
                'message',
                event => listener(event.data),
            ),
            onFailure: listener => {
                worker.addEventListener('error', event => {
                    listener(new Error(event.message || 'embedded compiler worker failed'));
                });
                worker.addEventListener('messageerror', () => {
                    listener(new Error('embedded compiler worker returned an unreadable message'));
                });
            },
            postMessage: (message, transfer) => worker.postMessage(message, [...transfer]),
            terminate: () => worker.terminate(),
        };
        return new EmbeddedCompilerBrowserWorkerClient(
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
