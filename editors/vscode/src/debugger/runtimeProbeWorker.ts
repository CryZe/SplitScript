import { parentPort } from 'node:worker_threads';

import { neutralImport } from './asr/neutralImports';

interface ProbeRequest {
    wasm: ArrayBuffer;
    runUpdate: boolean;
}

interface ProbeReady {
    type: 'ready';
    imports: string[];
    memoryBytes: number;
}

interface ProbeUpdated {
    type: 'updated';
}

interface ProbeFailure {
    type: 'failure';
    message: string;
    stack?: string;
}

type ProbeResponse = ProbeReady | ProbeUpdated | ProbeFailure;

const port = parentPort;
if (port === null) {
    throw new Error('the runtime probe must run in a Node worker');
}
const workerPort = port;

workerPort.once('message', (request: ProbeRequest) => {
    void run(request).catch((error: unknown) => {
        const failure: ProbeFailure = {
            type: 'failure',
            message: error instanceof Error ? error.message : String(error),
            stack: error instanceof Error ? error.stack : undefined,
        };
        workerPort.postMessage(failure satisfies ProbeResponse);
    });
});

async function run(request: ProbeRequest): Promise<void> {
    const module = await WebAssembly.compile(request.wasm);
    const moduleImports = WebAssembly.Module.imports(module);
    const imports = stubImports(moduleImports);
    const instance = await WebAssembly.instantiate(module, imports);
    const memory = instance.exports.memory;
    if (!(memory instanceof WebAssembly.Memory)) {
        throw new Error('the ASR module does not export its memory');
    }

    const initialize = instance.exports._initialize;
    if (typeof initialize === 'function') {
        initialize();
    }

    workerPort.postMessage({
        type: 'ready',
        imports: moduleImports.map(entry => `${entry.module}.${entry.name}`),
        memoryBytes: memory.buffer.byteLength,
    } satisfies ProbeResponse);

    if (!request.runUpdate) {
        return;
    }
    const update = instance.exports.update;
    if (typeof update !== 'function') {
        throw new Error('the ASR module does not export update');
    }
    update();
    workerPort.postMessage({ type: 'updated' } satisfies ProbeResponse);
}

function stubImports(entries: WebAssembly.ModuleImportDescriptor[]): WebAssembly.Imports {
    const imports: Record<string, Record<string, WebAssembly.ImportValue>> = {};
    for (const entry of entries) {
        if (entry.kind !== 'function') {
            throw new Error(`unsupported probe import ${entry.module}.${entry.name} (${entry.kind})`);
        }
        const namespace = imports[entry.module] ??= {};
        namespace[entry.name] = neutralImport(`${entry.module}.${entry.name}`);
    }
    return imports;
}
