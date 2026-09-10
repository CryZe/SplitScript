import { Worker } from 'node:worker_threads';

import type {
    RuntimeLogMessage,
    RuntimeRequest,
    RuntimeResponse,
    RuntimeSnapshot,
    SettingMapSnapshot,
} from './runtimeProtocol';

export interface RuntimeClientCallbacks {
    snapshot(snapshot: RuntimeSnapshot): void;
    log(message: RuntimeLogMessage): void;
    failure(error: Error): void;
}

export class RuntimeClient {
    private worker: Worker | undefined;
    private intentionalTermination = false;
    private settings: SettingMapSnapshot | undefined;
    private nextRequestId = 1;
    private readonly memoryDumps = new Map<
        number,
        { resolve(bytes: Uint8Array): void; reject(error: Error): void }
    >();

    public constructor(
        private readonly workerPath: string,
        private readonly nativeModulePath: string,
        private readonly callbacks: RuntimeClientCallbacks,
    ) {}

    public async launch(wasm: Uint8Array, program: string, scriptPath?: string): Promise<void> {
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
            scriptPath,
            settings: this.settings,
            nativeModulePath: this.nativeModulePath,
        }, [owned.buffer]);
        await ready;
    }

    public timerCommand(command: 'start' | 'reset'): void {
        if (this.worker === undefined) {
            return;
        }
        this.post({ type: 'timerCommand', command });
    }

    public setSetting(key: string, value: boolean | string): void {
        if (this.worker !== undefined) this.post({ type: 'setSetting', key, value });
    }

    public clearSettings(): void {
        if (this.worker !== undefined) this.post({ type: 'clearSettings' });
    }

    public resetStatistics(): void {
        if (this.worker !== undefined) this.post({ type: 'resetStatistics' });
    }

    public dumpMemory(): Promise<Uint8Array> {
        if (this.worker === undefined) {
            return Promise.reject(new Error('the ASR runtime worker is not running'));
        }
        const requestId = this.nextRequestId++;
        return new Promise((resolve, reject) => {
            this.memoryDumps.set(requestId, { resolve, reject });
            try {
                this.post({ type: 'dumpMemory', requestId });
            } catch (error) {
                this.memoryDumps.delete(requestId);
                reject(asError(error));
            }
        });
    }

    public async terminate(): Promise<void> {
        const worker = this.worker;
        if (worker === undefined) {
            return;
        }
        this.intentionalTermination = true;
        this.worker = undefined;
        this.rejectMemoryDumps(new Error('the ASR runtime worker stopped before dumping memory'));
        const stopped = new Promise<void>(resolve => {
            const onMessage = (message: RuntimeResponse) => {
                if (message.type === 'stopped') {
                    worker.off('message', onMessage);
                    resolve();
                }
            };
            worker.on('message', onMessage);
            setTimeout(() => {
                worker.off('message', onMessage);
                resolve();
            }, 250).unref();
        });
        worker.postMessage({ type: 'shutdown' } satisfies RuntimeRequest);
        await stopped;
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
            this.settings = message.snapshot.settings.map;
            this.callbacks.snapshot(message.snapshot);
        } else if (message.type === 'log') {
            this.callbacks.log(message);
        } else if (message.type === 'failure') {
            this.handleFailure(runtimeError(message));
        } else if (message.type === 'memoryDump') {
            const pending = this.memoryDumps.get(message.requestId);
            this.memoryDumps.delete(message.requestId);
            pending?.resolve(new Uint8Array(message.bytes));
        } else if (message.type === 'memoryDumpFailure') {
            const pending = this.memoryDumps.get(message.requestId);
            this.memoryDumps.delete(message.requestId);
            pending?.reject(new Error(message.message));
        }
    }

    private handleFailure(error: Error): void {
        this.rejectMemoryDumps(error);
        if (!this.intentionalTermination) {
            this.callbacks.failure(error);
        }
    }

    private rejectMemoryDumps(error: Error): void {
        for (const pending of this.memoryDumps.values()) pending.reject(error);
        this.memoryDumps.clear();
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

function asError(error: unknown): Error {
    return error instanceof Error ? error : new Error(String(error));
}
