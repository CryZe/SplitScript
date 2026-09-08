import { Worker } from 'node:worker_threads';

import type {
    RuntimeLogMessage,
    RuntimeRequest,
    RuntimeResponse,
    RuntimeSnapshot,
} from './runtimeProtocol';

export interface RuntimeClientCallbacks {
    snapshot(snapshot: RuntimeSnapshot): void;
    log(message: RuntimeLogMessage): void;
    failure(error: Error): void;
}

export class RuntimeClient {
    private worker: Worker | undefined;
    private intentionalTermination = false;

    public constructor(
        private readonly workerPath: string,
        private readonly callbacks: RuntimeClientCallbacks,
    ) {}

    public async launch(wasm: Uint8Array, program: string): Promise<void> {
        await this.terminate();
        const worker = new Worker(this.workerPath);
        this.worker = worker;
        this.intentionalTermination = false;

        const ready = new Promise<void>((resolve, reject) => {
            const onMessage = (message: RuntimeResponse) => {
                if (message.type === 'ready') {
                    cleanup();
                    resolve();
                } else if (message.type === 'failure') {
                    cleanup();
                    reject(runtimeError(message));
                }
            };
            const onError = (error: Error) => {
                cleanup();
                reject(error);
            };
            const cleanup = () => {
                worker.off('message', onMessage);
                worker.off('error', onError);
            };
            worker.on('message', onMessage);
            worker.on('error', onError);
        });

        worker.on('message', message => this.handleMessage(message as RuntimeResponse));
        worker.on('error', error => this.handleFailure(error));
        worker.on('exit', code => {
            if (this.worker === worker) {
                this.worker = undefined;
            }
            if (!this.intentionalTermination && code !== 0) {
                this.handleFailure(new Error(`ASR runtime worker exited with code ${code}`));
            }
        });

        const owned = new Uint8Array(wasm.length);
        owned.set(wasm);
        this.post({
            type: 'launch',
            wasm: owned.buffer,
            program,
        }, [owned.buffer]);
        await ready;
    }

    public timerCommand(command: 'start' | 'reset'): void {
        if (this.worker === undefined) {
            return;
        }
        this.post({ type: 'timerCommand', command });
    }

    public async terminate(): Promise<void> {
        const worker = this.worker;
        if (worker === undefined) {
            return;
        }
        this.intentionalTermination = true;
        this.worker = undefined;
        await worker.terminate();
    }

    public dispose(): void {
        void this.terminate();
    }

    private post(message: RuntimeRequest, transfer: readonly ArrayBuffer[] = []): void {
        const worker = this.worker;
        if (worker === undefined) {
            throw new Error('the ASR runtime worker is not running');
        }
        worker.postMessage(message, [...transfer]);
    }

    private handleMessage(message: RuntimeResponse): void {
        if (message.type === 'snapshot') {
            this.callbacks.snapshot(message.snapshot);
        } else if (message.type === 'log') {
            this.callbacks.log(message);
        } else if (message.type === 'failure') {
            this.handleFailure(runtimeError(message));
        }
    }

    private handleFailure(error: Error): void {
        if (!this.intentionalTermination) {
            this.callbacks.failure(error);
        }
    }
}

function runtimeError(message: Extract<RuntimeResponse, { type: 'failure' }>): Error {
    const error = new Error(message.message);
    error.name = 'AsrRuntimeError';
    if (message.stack !== undefined) {
        error.stack = message.stack;
    }
    return error;
}
