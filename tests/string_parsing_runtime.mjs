import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/string_parsing_runtime.mjs <string-parsing.wasm>");
}

const decoder = new TextDecoder();
let instance;
let observed;
let dynamicObserved;
let dynamicText = "0";
let dynamicExpected = 0;
let dynamicValid = true;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 0,
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    process_read(_process, address, destination, size) {
        const memory = new Uint8Array(instance.exports.memory.buffer);
        const view = new DataView(instance.exports.memory.buffer);
        if (address === 0x100n && size === 1024) {
            memory.fill(0, destination, destination + size);
            memory.set(new TextEncoder().encode(dynamicText), destination);
            return 1;
        }
        if (address === 0x500n && size === 8) {
            view.setFloat64(destination, dynamicExpected, true);
            return 1;
        }
        if (address === 0x508n && size === 1) {
            view.setUint8(destination, dynamicValid ? 1 : 0);
            return 1;
        }
        throw new Error(`unexpected process read: ${address.toString(16)} (${size} bytes)`);
    },
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        if (text(keyPointer, keyLength) === "String Parsing") {
            observed = text(valuePointer, valueLength);
        } else if (text(keyPointer, keyLength) === "String Parsing Dynamic") {
            dynamicObserved = text(valuePointer, valueLength);
        }
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
instance.exports.update();
instance.exports.update();

const expected = "255";
if (observed !== expected) {
    throw new Error(`unexpected string-parsing output: ${JSON.stringify({ expected, observed })}`);
}


const invalid = ["", "+", "-", ".", "1e", "1e+", " 1", "1 ", "1_0", "NaNx"];
const valid = [
    "NaN", "-nan", "inf", "+Infinity", "-INFINITY", "-0", ".5", "5.",
    "2.2250738585072014e-308", "4.9406564584124654e-324",
    "1.7976931348623157e308", "1.7976931348623159e308",
    "2.47032822920623272e-324",
    "0.333333333333333333333333333333333333333333333333333333333333333333333333",
];
valid.push(`0.${"3".repeat(1000)}`);
valid.push(`1${"0".repeat(1000)}e-1000`);

let randomState = 0x9e3779b9;
const random = () => {
    randomState ^= randomState << 13;
    randomState ^= randomState >>> 17;
    randomState ^= randomState << 5;
    return randomState >>> 0;
};
for (let caseIndex = 0; caseIndex < 2000; caseIndex += 1) {
    const length = 1 + random() % 80;
    let digits = "";
    for (let index = 0; index < length; index += 1) {
        digits += String(random() % 10);
    }
    const point = random() % (length + 1);
    const mantissa = `${digits.slice(0, point)}.${digits.slice(point)}`;
    const exponent = Number(random() % 801) - 400;
    valid.push(`${random() & 1 ? "-" : ""}${mantissa}e${exponent}`);
}

for (const source of valid) {
    dynamicText = source;
    const lower = source.toLowerCase();
    const unsigned = lower.replace(/^[+-]/, "");
    dynamicExpected = unsigned === "inf" || unsigned === "infinity"
        ? (lower.startsWith("-") ? -Infinity : Infinity)
        : Number(source);
    dynamicValid = true;
    instance.exports.update();
    if (dynamicObserved !== "true") {
        throw new Error(`correctly-rounded parse mismatch for ${JSON.stringify(source)}`);
    }
}

for (const source of invalid) {
    dynamicText = source;
    dynamicExpected = 0;
    dynamicValid = false;
    instance.exports.update();
    if (dynamicObserved !== "true") {
        throw new Error(`invalid float spelling was accepted: ${JSON.stringify(source)}`);
    }
}

console.log(JSON.stringify({ observed, decimalCases: valid.length, invalidCases: invalid.length }));
