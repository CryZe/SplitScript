// Offline reference experiment; Binaryen is not part of the compiler pipeline.
// node scripts/binaryen-review.mjs <wasm-opt> [splitc] [output-directory]
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { performance } from "node:perf_hooks";

const [optimizer, compiler = "target/max-opt/splitc.exe", output = "target/performance-review/binaryen-132"] = process.argv.slice(2);
if (!optimizer) throw new Error("usage: node scripts/binaryen-review.mjs <wasm-opt> [splitc] [output-directory]");
fs.mkdirSync(output, { recursive: true });

function run(executable, args) {
    const result = spawnSync(executable, args, { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 });
    if (result.error || result.status !== 0) {
        throw new Error(`${executable} ${args.join(" ")}\n${result.error ?? result.stderr ?? result.stdout}`);
    }
    return result.stdout;
}

function sections(filename) {
    const bytes = fs.readFileSync(filename);
    let offset = 8;
    const leb = () => {
        let value = 0;
        let shift = 0;
        let byte;
        do {
            byte = bytes[offset++];
            value += (byte & 127) * 2 ** shift;
            shift += 7;
        } while (byte & 128);
        return value;
    };
    const result = { bytes: bytes.length, custom: 0, sections: {}, functionBodies: [] };
    while (offset < bytes.length) {
        const start = offset;
        const id = bytes[offset++];
        const size = leb();
        const end = offset + size;
        if (id === 0) result.custom += end - start;
        else result.sections[id] = (result.sections[id] ?? 0) + end - start;
        if (id === 10) {
            const count = leb();
            for (let i = 0; i < count; i++) {
                const bodySize = leb();
                result.functionBodies.push(bodySize);
                offset += bodySize;
            }
        }
        offset = end;
    }
    result.noncustom = result.bytes - result.custom;
    return result;
}

const features = ["--enable-gc", "--enable-reference-types", "--enable-multivalue", "--enable-bulk-memory", "--enable-sign-ext", "--enable-nontrapping-float-to-int"];
const modes = {
    rewrite: [],
    peephole: ["--optimize-instructions"],
    O4: ["-O4"],
    Oz: ["-Oz"],
    closed: ["-O4", "--shrink-level=2", "--closed-world", "--converge"],
};
// Avoid a Windows Node 24 background-worker shutdown assertion in fixtures
// that call process.exit(). This is a semantic trace check, not a runtime benchmark.
const runtimeFlags = ["--single-threaded", "--no-wasm-async-compilation"];
// Closed-world is a separate ceiling experiment: these scripts expose only
// numeric host calls and linear memory, with no external GC-reference users.
const fixtures = [
    ["lunistice", "examples/lunistice.split", "tests/lunistice_runtime.mjs", [[], ["--dlc"], ["--transient-binding-read"], ["--mixed-layout"], ["--inherited-field"]]],
    ["minish_cap", "examples/minish_cap.split", "tests/minish_cap_runtime.mjs", [[], ["vba"]]],
    ["settings", "examples/lso_desktop_settings.split", "tests/settings_runtime.mjs", [[]]],
    ["cancellation", "examples/cancellation.split", "tests/cancellation_runtime.mjs", [[]]],
    ["managed", "tests/managed_instances_runtime.split", "tests/managed_instances_runtime.mjs", [[]]],
    ["managed_mono", "tests/managed_instances_mono_runtime.split", "tests/managed_instances_mono_runtime.mjs", [[]]],
    ["set", "tests/set_runtime.split", "tests/set_runtime.mjs", [[]]],
    ["map", "tests/map_runtime.split", "tests/map_runtime.mjs", [[]]],
];
const report = { optimizer: run(optimizer, ["--version"]).trim(), compiler, features, runtimeFlags, modes, fixtures: [] };
console.log("fixture\tmode\tbytes\tnoncustom\tcode\tfunctions\toptimizer_ms\truntime_cases");
for (const [name, source, harness, cases] of fixtures) {
    const original = path.join(output, `${name}.original.wasm`);
    run(compiler, [source, "--profile", "release", "--allow", "warnings", "-o", original]);
    run("wasm-tools", ["validate", "--features", "all", original]);
    const expected = cases.map(args => run(process.execPath, [...runtimeFlags, harness, original, ...args]));
    const entry = { name, source, harness, cases, original: sections(original), variants: {} };
    console.log(`${name}\toriginal\t${entry.original.bytes}\t${entry.original.noncustom}\t${entry.original.sections[10]}\t${entry.original.functionBodies.length}\t-\t${cases.length}`);
    for (const [mode, options] of Object.entries(modes)) {
        const optimized = path.join(output, `${name}.${mode}.wasm`);
        const start = performance.now();
        run(optimizer, [original, ...features, ...options, "-o", optimized]);
        const elapsedMs = performance.now() - start;
        run("wasm-tools", ["validate", "--features", "all", optimized]);
        for (let i = 0; i < cases.length; i++) {
            const actual = run(process.execPath, [...runtimeFlags, harness, optimized, ...cases[i]]);
            if (actual !== expected[i]) throw new Error(`${name}/${mode}: runtime trace differs for ${cases[i]}`);
        }
        const result = { ...sections(optimized), elapsedMs };
        entry.variants[mode] = result;
        console.log(`${name}\t${mode}\t${result.bytes}\t${result.noncustom}\t${result.sections[10]}\t${result.functionBodies.length}\t${elapsedMs.toFixed(1)}\t${cases.length}`);
    }
    report.fixtures.push(entry);
    fs.writeFileSync(path.join(output, "results.json"), JSON.stringify(report, null, 2) + "\n");
}
