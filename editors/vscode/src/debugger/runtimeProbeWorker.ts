import { parentPort } from 'node:worker_threads';

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
        namespace[entry.name] = BIGINT_RESULTS.has(`${entry.module}.${entry.name}`)
            ? () => 0n
            : () => 0;
    }
    return imports;
}

// JavaScript must return BigInt for a WebAssembly i64 result. The production
// host will define exact signatures; this probe only needs type-correct neutral
// values to validate V8 compilation, instantiation, initialization, and update.
const BIGINT_RESULTS = new Set([
    'env.process_attach',
    'env.process_attach_by_pid',
    'env.process_get_memory_range_address',
    'env.process_get_memory_range_count',
    'env.process_get_memory_range_flags',
    'env.process_get_memory_range_size',
    'env.process_get_module_address',
    'env.process_get_module_size',
    'env.setting_value_copy',
    'env.setting_value_get_i64',
    'env.setting_value_get_list',
    'env.setting_value_get_map',
    'env.setting_value_new_bool',
    'env.setting_value_new_f64',
    'env.setting_value_new_i64',
    'env.setting_value_new_list',
    'env.setting_value_new_map',
    'env.setting_value_new_string',
    'env.settings_list_copy',
    'env.settings_list_get',
    'env.settings_list_new',
    'env.settings_map_copy',
    'env.settings_map_get',
    'env.settings_map_get_value_by_index',
    'env.settings_map_load',
    'env.settings_map_new',
]);
