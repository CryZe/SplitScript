import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/string_parsing_runtime.mjs <string-parsing.wasm>");
}

const decoder = new TextDecoder();
let instance;
let observed;
let dynamicObserved;
let radixObserved;
let dynamicRadixObserved;
let dynamicText = "0";
let dynamicExpected = 0;
let dynamicValid = true;
let dynamicRadixUnsigned = 0n;
let dynamicRadixSigned = 0n;
let dynamicRadix = 10;
let dynamicRadixUsesSigned = false;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 0,
    runtime_set_tick_rate() {},
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
        if (address === 0x510n && size === 8) {
            view.setBigUint64(destination, dynamicRadixUnsigned, true);
            return 1;
        }
        if (address === 0x518n && size === 8) {
            view.setBigInt64(destination, dynamicRadixSigned, true);
            return 1;
        }
        if (address === 0x520n && size === 4) {
            view.setUint32(destination, dynamicRadix, true);
            return 1;
        }
        if (address === 0x524n && size === 1) {
            view.setUint8(destination, dynamicRadixUsesSigned ? 1 : 0);
            return 1;
        }
        throw new Error(`unexpected process read: ${address.toString(16)} (${size} bytes)`);
    },
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        if (text(keyPointer, keyLength) === "String Parsing") {
            observed = text(valuePointer, valueLength);
        } else if (text(keyPointer, keyLength) === "String Parsing Dynamic") {
            dynamicObserved = text(valuePointer, valueLength);
        } else if (text(keyPointer, keyLength) === "Integer Radix") {
            radixObserved = text(valuePointer, valueLength);
        } else if (text(keyPointer, keyLength) === "Integer Radix Dynamic") {
            dynamicRadixObserved = text(valuePointer, valueLength);
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

const expectedRadix = "0,11111111,ff,-80,-8000000000000000,3w5e11264sgsf,1234,ABCDEF,42,true";
if (radixObserved !== expectedRadix) {
    throw new Error(`unexpected integer-radix output: ${JSON.stringify({ expectedRadix, radixObserved })}`);
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

for (const radix of [0, 1, 37, 0xffff_ffff]) {
    dynamicRadix = radix;
    dynamicRadixUsesSigned = false;
    instance.exports.update();
    if (dynamicRadixObserved !== "error") {
        throw new Error(`invalid integer radix was accepted: ${JSON.stringify({ radix, dynamicRadixObserved })}`);
    }
}

const unsignedRadixCases = [0n, 1n, 0xffff_ffffn, 0xffff_ffff_ffff_ffffn];
for (let index = 0; index < 16; index += 1) {
    unsignedRadixCases.push((BigInt(random()) << 32n) | BigInt(random()));
}
const signedRadixCases = [
    -0x8000_0000_0000_0000n,
    -1n,
    0n,
    1n,
    0x7fff_ffff_ffff_ffffn,
    ...unsignedRadixCases.map((value) => BigInt.asIntN(64, value)),
];

for (let radix = 2; radix <= 36; radix += 1) {
    dynamicRadix = radix;
    dynamicRadixUsesSigned = false;
    for (const value of unsignedRadixCases) {
        dynamicRadixUnsigned = value;
        instance.exports.update();
        const expectedValue = value.toString(radix);
        if (dynamicRadixObserved !== expectedValue) {
            throw new Error(`unsigned integer radix mismatch: ${JSON.stringify({ radix, value: value.toString(), expectedValue, dynamicRadixObserved })}`);
        }
    }

    dynamicRadixUsesSigned = true;
    for (const value of signedRadixCases) {
        dynamicRadixSigned = value;
        instance.exports.update();
        const expectedValue = value.toString(radix);
        if (dynamicRadixObserved !== expectedValue) {
            throw new Error(`signed integer radix mismatch: ${JSON.stringify({ radix, value: value.toString(), expectedValue, dynamicRadixObserved })}`);
        }
    }
}

console.log(JSON.stringify({ observed, radixObserved, decimalCases: valid.length, invalidCases: invalid.length }));
