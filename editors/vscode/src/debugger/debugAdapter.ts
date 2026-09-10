import * as vscode from 'vscode';

import type { EmbeddedCompilerClient } from '../compilerTasks';
import {
    DebugRuntimeSession,
    type SplitScriptLaunchConfiguration,
} from './debugRuntimeSession';
import type { RuntimeLogMessage, RuntimeSnapshot } from './runtimeProtocol';

interface DapRequest {
    seq: number;
    type: 'request';
    command: string;
    arguments?: Record<string, unknown>;
}

interface DapResponse {
    seq: number;
    type: 'response';
    request_seq: number;
    success: boolean;
    command: string;
    message?: string;
    body?: unknown;
}

interface DapEvent {
    seq: number;
    type: 'event';
    event: string;
    body?: unknown;
}

export interface DebugAdapterHost {
    createCompiler(): Promise<EmbeddedCompilerClient>;
    snapshot(adapter: SplitScriptDebugAdapter, snapshot: RuntimeSnapshot | undefined): void;
    log(adapter: SplitScriptDebugAdapter, message: RuntimeLogMessage): void;
    active(adapter: SplitScriptDebugAdapter): void;
    stopped(adapter: SplitScriptDebugAdapter): void;
}

export class SplitScriptDebugAdapter implements vscode.DebugAdapter {
    private readonly messages = new vscode.EventEmitter<DapResponse | DapEvent>();
    private readonly runtime: DebugRuntimeSession;
    private sequence = 1;
    private terminated = false;

    public readonly onDidSendMessage = this.messages.event;

    public constructor(
        workerPath: string,
        nativeModulePath: string,
        private readonly host: DebugAdapterHost,
    ) {
        this.runtime = new DebugRuntimeSession(
            workerPath,
            nativeModulePath,
            () => host.createCompiler(),
            {
                snapshot: snapshot => host.snapshot(this, snapshot),
                log: message => {
                    host.log(this, message);
                    this.output(message);
                },
                failure: error => this.runtimeFailure(error),
            },
        );
    }

    public handleMessage(message: DapRequest): void {
        void this.handleRequest(message).catch((error: unknown) => {
            this.respond(message, undefined, asError(error));
            if (message.command === 'launch') {
                this.sendTerminated();
            }
        });
    }

    public async restart(): Promise<void> {
        await this.runtime.reload();
    }

    public timerCommand(command: 'start' | 'reset'): void {
        this.runtime.timerCommand(command);
    }

    public setSetting(key: string, value: boolean | string): void {
        this.runtime.setSetting(key, value);
    }

    public clearSettings(): void {
        this.runtime.clearSettings();
    }

    public resetStatistics(): void {
        this.runtime.resetStatistics();
    }

    public dumpMemory(): Promise<Uint8Array> {
        return this.runtime.dumpMemory();
    }

    public async stop(): Promise<void> {
        await this.runtime.stop();
        this.sendTerminated();
    }

    public dispose(): void {
        this.host.stopped(this);
        this.runtime.dispose();
        this.messages.dispose();
    }

    private async handleRequest(request: DapRequest): Promise<void> {
        switch (request.command) {
            case 'initialize':
                this.respond(request, {
                    supportsConfigurationDoneRequest: true,
                    supportsRestartRequest: true,
                    supportsTerminateRequest: true,
                });
                this.event('initialized');
                break;
            case 'launch': {
                const configuration = request.arguments as SplitScriptLaunchConfiguration;
                this.host.active(this);
                await this.runtime.launch(configuration);
                this.respond(request);
                this.event('process', {
                    name: configuration.program,
                    isLocalProcess: true,
                    startMethod: 'launch',
                });
                break;
            }
            case 'configurationDone':
            case 'setExceptionBreakpoints':
                this.respond(request);
                break;
            case 'threads':
                this.respond(request, { threads: [{ id: 1, name: 'ASR Runtime' }] });
                break;
            case 'stackTrace':
                this.respond(request, { stackFrames: [], totalFrames: 0 });
                break;
            case 'scopes':
            case 'variables':
                this.respond(request, request.command === 'scopes' ? { scopes: [] } : { variables: [] });
                break;
            case 'restart':
                await this.restart();
                this.respond(request);
                break;
            case 'disconnect':
            case 'terminate':
                await this.runtime.stop();
                this.respond(request);
                this.sendTerminated();
                break;
            default:
                this.respond(request, undefined, new Error(
                    `SplitScript debugger does not support the ${request.command} request yet.`,
                ));
                break;
        }
    }

    private runtimeFailure(error: Error): void {
        const message: RuntimeLogMessage = {
            type: 'log',
            timestamp: new Date().toISOString(),
            source: 'runtime',
            level: 'error',
            message: error.stack ?? error.message,
        };
        this.host.log(this, message);
        this.output(message);
        this.sendTerminated();
    }

    private output(message: RuntimeLogMessage): void {
        const category = message.level === 'error' ? 'stderr' : 'console';
        this.event('output', {
            category,
            output: `[${message.timestamp}][${message.source}][${message.level}] ${message.message}\n`,
        });
    }

    private respond(request: DapRequest, body?: unknown, error?: Error): void {
        this.messages.fire({
            seq: this.sequence++,
            type: 'response',
            request_seq: request.seq,
            success: error === undefined,
            command: request.command,
            message: error?.message,
            body,
        });
    }

    private event(event: string, body?: unknown): void {
        this.messages.fire({
            seq: this.sequence++,
            type: 'event',
            event,
            body,
        });
    }

    private sendTerminated(): void {
        if (this.terminated) {
            return;
        }
        this.terminated = true;
        this.host.stopped(this);
        this.event('terminated');
    }
}

function asError(error: unknown): Error {
    return error instanceof Error ? error : new Error(String(error));
}
