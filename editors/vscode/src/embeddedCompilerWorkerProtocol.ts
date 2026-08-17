import type {
    EmbeddedCompileRequest,
    EmbeddedCompileResponse,
} from './embeddedCompiler';

export interface InitializeWorkerRequest {
    id: number;
    kind: 'initialize';
    moduleBytes: ArrayBuffer;
}

export interface CompileWorkerRequest {
    id: number;
    kind: 'compile';
    request: EmbeddedCompileRequest;
}

export interface CancelWorkerRequest {
    id: number;
    kind: 'cancel';
    targetId: number;
}

export type EmbeddedCompilerWorkerRequest =
    | InitializeWorkerRequest
    | CompileWorkerRequest
    | CancelWorkerRequest;

export interface EmbeddedCompilerWorkerSuccess {
    id: number;
    ok: true;
    response?: EmbeddedCompileResponse;
}

export interface EmbeddedCompilerWorkerFailure {
    id: number;
    ok: false;
    error: {
        name: string;
        message: string;
        code?: string;
    };
}

export type EmbeddedCompilerWorkerResponse =
    | EmbeddedCompilerWorkerSuccess
    | EmbeddedCompilerWorkerFailure;
