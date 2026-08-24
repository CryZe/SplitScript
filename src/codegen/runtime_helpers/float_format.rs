//! Shortest correctly rounded floating-point formatting for generated Wasm.
//!
//! The conversion is adapted from David Tolnay's `zmij` crate, which is an
//! MIT-licensed Rust port of Victor Zverovich's Schubfach implementation:
//! <https://github.com/dtolnay/zmij>. We deliberately use zmij's compact,
//! correctness-first Schubfach path rather than its architecture-specific fast
//! paths. SplitScript emits Wasm-GC directly, so the Rust crate cannot be
//! linked into generated modules; this module preserves the algorithm while
//! adapting its output to SplitScript's GC-backed `String` representation.
//!
//! zmij is distributed under the MIT license:
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! of this software and associated documentation files (the "Software"), to
//! deal in the Software without restriction, including without limitation the
//! rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
//! sell copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in
//! all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
//! FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
//! IN THE SOFTWARE.

use wasm_encoder::{BlockType, Function, Instruction, MemArg, ValType};

use crate::stdlib::StdlibTypeId;

use super::super::memory_plan::ScratchRegion;
use super::super::{GcLayout, Type};

const POW10S: [u64; 28] = [
    0x8000000000000000,
    0xa000000000000000,
    0xc800000000000000,
    0xfa00000000000000,
    0x9c40000000000000,
    0xc350000000000000,
    0xf424000000000000,
    0x9896800000000000,
    0xbebc200000000000,
    0xee6b280000000000,
    0x9502f90000000000,
    0xba43b74000000000,
    0xe8d4a51000000000,
    0x9184e72a00000000,
    0xb5e620f480000000,
    0xe35fa931a0000000,
    0x8e1bc9bf04000000,
    0xb1a2bc2ec5000000,
    0xde0b6b3a76400000,
    0x8ac7230489e80000,
    0xad78ebc5ac620000,
    0xd8d726b7177a8000,
    0x878678326eac9000,
    0xa968163f0a57b400,
    0xd3c21bcecceda100,
    0x84595161401484a0,
    0xa56fa5b99019a5c8,
    0xcecb8f27f4200f3a,
];

const HIGH_PARTS: [(u64, u64); 23] = [
    (0xaf8e5410288e1b6f, 0x07ecf0ae5ee44dda),
    (0xb1442798f49ffb4a, 0x99cd11cfdf41779d),
    (0xb2fe3f0b8599ef07, 0x861fa7e6dcb4aa15),
    (0xb4bca50b065abe63, 0x0fed077a756b53aa),
    (0xb67f6455292cbf08, 0x1a3bc84c17b1d543),
    (0xb84687c269ef3bfb, 0x3d5d514f40eea742),
    (0xba121a4650e4ddeb, 0x92f34d62616ce413),
    (0xbbe226efb628afea, 0x890489f70a55368c),
    (0xbdb6b8e905cb600f, 0x5400e987bbc1c921),
    (0xbf8fdb78849a5f96, 0xde98520472bdd034),
    (0xc16d9a0095928a27, 0x75b7053c0f178294),
    (0xc350000000000000, 0x0000000000000000),
    (0xc5371912364ce305, 0x6c28000000000000),
    (0xc722f0ef9d80aad6, 0x424d3ad2b7b97ef6),
    (0xc913936dd571c84c, 0x03bc3a19cd1e38ea),
    (0xcb090c8001ab551c, 0x5cadf5bfd3072cc6),
    (0xcd036837130890a1, 0x36dba887c37a8c10),
    (0xcf02b2c21207ef2e, 0x94f967e45e03f4bc),
    (0xd106f86e69d785c7, 0xe13336d701beba52),
    (0xd31045a8341ca07c, 0x1ede48111209a051),
    (0xd51ea6fa85785631, 0x552a74227f3ea566),
    (0xd732290fbacaf133, 0xa97c177947ad4096),
    (0xd94ad8b1c7380874, 0x18375281ae7822bc),
];

const FIXUPS: [u32; 20] = [
    0x05271b1f, 0x00000c20, 0x00003200, 0x12100020, 0x00000000, 0x06000000, 0xc16409c0, 0xaf26700f,
    0xeb987b07, 0x0000000d, 0x00000000, 0x66fbfffe, 0xb74100ec, 0xa0669fe8, 0xedb21280, 0x00000686,
    0x0a021200, 0x29b89c20, 0x08bc0eda, 0x00000000,
];

const POW10_COUNT: usize = 617;
pub(super) const POW10_BYTES: usize = POW10_COUNT * 16;

const fn pow10_significand(i: usize) -> (u64, u64) {
    let m = POW10S[(i + 11) % 28];
    let (h_hi, h_lo) = HIGH_PARTS[(i + 11) / 28];
    let h1 = ((h_lo as u128 * m as u128) >> 64) as u64;
    let c0 = h_lo.wrapping_mul(m);
    let c1 = h1.wrapping_add(h_hi.wrapping_mul(m));
    let c2 = (c1 < h1) as u64 + ((h_hi as u128 * m as u128) >> 64) as u64;
    let (hi, mut lo) = if c2 >> 63 != 0 {
        (c2, c1)
    } else {
        ((c2 << 1) | (c1 >> 63), (c1 << 1) | (c0 >> 63))
    };
    lo -= ((FIXUPS[i >> 5] >> (i & 31)) & 1) as u64;
    (hi, lo)
}

pub(in crate::codegen) fn pow10_significands_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(POW10_BYTES);
    for index in 0..POW10_COUNT {
        let (hi, lo) = pow10_significand(index);
        bytes.extend_from_slice(&hi.to_le_bytes());
        bytes.extend_from_slice(&lo.to_le_bytes());
    }
    bytes
}

const fn memarg(align: u32) -> MemArg {
    MemArg {
        offset: 0,
        align,
        memory_index: 0,
    }
}

/// Unsigned 64-by-64 multiplication returning the low then high halves.
pub(super) fn compile_mul128() -> Function {
    let mut function = Function::new([(10, ValType::I64)]);
    let x = 0;
    let y = 1;
    let x0 = 2;
    let x1 = 3;
    let y0 = 4;
    let y1 = 5;
    let w0 = 6;
    let t = 7;
    let w1 = 8;
    let w2 = 9;
    let lo = 10;
    let hi = 11;
    let mask = 0xffff_ffffu64 as i64;

    function
        .instruction(&Instruction::LocalGet(x))
        .instruction(&Instruction::I64Const(mask))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::LocalSet(x0))
        .instruction(&Instruction::LocalGet(x))
        .instruction(&Instruction::I64Const(32))
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::LocalSet(x1))
        .instruction(&Instruction::LocalGet(y))
        .instruction(&Instruction::I64Const(mask))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::LocalSet(y0))
        .instruction(&Instruction::LocalGet(y))
        .instruction(&Instruction::I64Const(32))
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::LocalSet(y1))
        .instruction(&Instruction::LocalGet(x0))
        .instruction(&Instruction::LocalGet(y0))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::LocalSet(w0))
        .instruction(&Instruction::LocalGet(x1))
        .instruction(&Instruction::LocalGet(y0))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::LocalGet(w0))
        .instruction(&Instruction::I64Const(32))
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalTee(t))
        .instruction(&Instruction::I64Const(mask))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::LocalSet(w1))
        .instruction(&Instruction::LocalGet(t))
        .instruction(&Instruction::I64Const(32))
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::LocalSet(w2))
        .instruction(&Instruction::LocalGet(x0))
        .instruction(&Instruction::LocalGet(y1))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::LocalGet(w1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(w1))
        .instruction(&Instruction::LocalGet(w1))
        .instruction(&Instruction::I64Const(32))
        .instruction(&Instruction::I64Shl)
        .instruction(&Instruction::LocalGet(w0))
        .instruction(&Instruction::I64Const(mask))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::I64Or)
        .instruction(&Instruction::LocalSet(lo))
        .instruction(&Instruction::LocalGet(x1))
        .instruction(&Instruction::LocalGet(y1))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::LocalGet(w2))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(w1))
        .instruction(&Instruction::I64Const(32))
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(hi))
        .instruction(&Instruction::LocalGet(lo))
        .instruction(&Instruction::LocalGet(hi))
        .instruction(&Instruction::End);
    function
}

/// High 128 bits of a 128-by-64 product, returned high then low.
pub(super) fn compile_mul192_hi128(mul128: u32) -> Function {
    let mut function = Function::new([(6, ValType::I64)]);
    let x_hi = 0;
    let x_lo = 1;
    let y = 2;
    let p_lo = 3;
    let p_hi = 4;
    let q_lo = 5;
    let q_hi = 6;
    let lo = 7;
    let carry = 8;

    function
        .instruction(&Instruction::LocalGet(x_hi))
        .instruction(&Instruction::LocalGet(y))
        .instruction(&Instruction::Call(mul128))
        .instruction(&Instruction::LocalSet(p_hi))
        .instruction(&Instruction::LocalSet(p_lo))
        .instruction(&Instruction::LocalGet(x_lo))
        .instruction(&Instruction::LocalGet(y))
        .instruction(&Instruction::Call(mul128))
        .instruction(&Instruction::LocalSet(q_hi))
        .instruction(&Instruction::LocalSet(q_lo))
        .instruction(&Instruction::LocalGet(p_lo))
        .instruction(&Instruction::LocalGet(q_hi))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalTee(lo))
        .instruction(&Instruction::LocalGet(p_lo))
        .instruction(&Instruction::I64LtU)
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalSet(carry))
        .instruction(&Instruction::LocalGet(p_hi))
        .instruction(&Instruction::LocalGet(carry))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(lo))
        .instruction(&Instruction::End);
    function
}

fn emit_decimal_prelude(function: &mut Function, pow10_base: i32, increment_low: bool) {
    let bin_exp = 1;
    let regular = 2;
    let dec_exp = 3;
    let shift = 4;
    let index = 5;
    let pow_hi = 8;
    let pow_lo = 9;

    function
        .instruction(&Instruction::LocalGet(bin_exp))
        .instruction(&Instruction::I32Const(315_653))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::LocalGet(regular))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::I32Const(131_072))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Const(20))
        .instruction(&Instruction::I32ShrS)
        .instruction(&Instruction::LocalSet(dec_exp))
        .instruction(&Instruction::LocalGet(bin_exp))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Const(217_707))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::I32Const(16))
        .instruction(&Instruction::I32ShrS)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(shift))
        .instruction(&Instruction::I32Const(pow10_base))
        .instruction(&Instruction::I32Const(292))
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::I32Shl)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalTee(index))
        .instruction(&Instruction::I64Load(memarg(3)))
        .instruction(&Instruction::LocalSet(pow_hi))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I64Load(memarg(3)))
        .instruction(&Instruction::LocalSet(pow_lo));
    let selected = if increment_low { pow_lo } else { pow_hi };
    function
        .instruction(&Instruction::LocalGet(selected))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(selected));
}

fn emit_mulhi_f32(function: &mut Function, operand: u32, mul128: u32) {
    let pow_hi = 8;
    let product_lo = 19;
    let product_hi = 20;
    function
        .instruction(&Instruction::LocalGet(pow_hi))
        .instruction(&Instruction::LocalGet(operand))
        .instruction(&Instruction::Call(mul128))
        .instruction(&Instruction::LocalSet(product_hi))
        .instruction(&Instruction::LocalSet(product_lo))
        .instruction(&Instruction::LocalGet(product_hi))
        .instruction(&Instruction::LocalGet(product_lo))
        .instruction(&Instruction::I64Const(32))
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Or)
        .instruction(&Instruction::I64Const(0xffff_ffffu64 as i64))
        .instruction(&Instruction::I64And);
}

fn emit_mulhi_f64(function: &mut Function, operand: u32, mul192: u32) {
    let pow_hi = 8;
    let pow_lo = 9;
    let product_hi = 19;
    let product_lo = 20;
    function
        .instruction(&Instruction::LocalGet(pow_hi))
        .instruction(&Instruction::LocalGet(pow_lo))
        .instruction(&Instruction::LocalGet(operand))
        .instruction(&Instruction::Call(mul192))
        .instruction(&Instruction::LocalSet(product_lo))
        .instruction(&Instruction::LocalSet(product_hi))
        .instruction(&Instruction::LocalGet(product_hi))
        .instruction(&Instruction::LocalGet(product_lo))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Or);
}

fn emit_decimal_finish(
    function: &mut Function,
    mask32: bool,
    mul128: Option<u32>,
    mul192: Option<u32>,
) {
    let sig = 0;
    let regular = 2;
    let dec_exp = 3;
    let shift = 4;
    let below_closer = 6;
    let below_in = 7;
    let shifted = 10;
    let lsb = 11;
    let lower = 12;
    let upper = 13;
    let shorter = 14;
    let product = 15;
    let scaled = 16;
    let below = 17;
    let above = 18;
    let mask = 0xffff_ffffu64 as i64;

    let narrow = |function: &mut Function| {
        if mask32 {
            function
                .instruction(&Instruction::I64Const(mask))
                .instruction(&Instruction::I64And);
        }
    };
    function
        .instruction(&Instruction::LocalGet(sig))
        .instruction(&Instruction::I64Const(2))
        .instruction(&Instruction::I64Shl);
    narrow(function);
    function
        .instruction(&Instruction::LocalSet(shifted))
        .instruction(&Instruction::LocalGet(sig))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::LocalSet(lsb))
        .instruction(&Instruction::LocalGet(shifted))
        .instruction(&Instruction::LocalGet(regular))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I64Sub);
    narrow(function);
    function
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Shl);
    narrow(function);
    function.instruction(&Instruction::LocalSet(lower));
    if let Some(mul192) = mul192 {
        emit_mulhi_f64(function, lower, mul192);
    } else {
        emit_mulhi_f32(
            function,
            lower,
            mul128.expect("f32 decimal conversion needs mul128"),
        );
    }
    function
        .instruction(&Instruction::LocalGet(lsb))
        .instruction(&Instruction::I64Add);
    narrow(function);
    function
        .instruction(&Instruction::LocalSet(lower))
        .instruction(&Instruction::LocalGet(shifted))
        .instruction(&Instruction::I64Const(2))
        .instruction(&Instruction::I64Add);
    narrow(function);
    function
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Shl);
    narrow(function);
    function.instruction(&Instruction::LocalSet(upper));
    if let Some(mul192) = mul192 {
        emit_mulhi_f64(function, upper, mul192);
    } else {
        emit_mulhi_f32(
            function,
            upper,
            mul128.expect("f32 decimal conversion needs mul128"),
        );
    }
    function
        .instruction(&Instruction::LocalGet(lsb))
        .instruction(&Instruction::I64Sub);
    narrow(function);
    function
        .instruction(&Instruction::LocalSet(upper))
        .instruction(&Instruction::LocalGet(upper))
        .instruction(&Instruction::I64Const(2))
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64DivU)
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::LocalTee(shorter))
        .instruction(&Instruction::I64Const(2))
        .instruction(&Instruction::I64Shl);
    narrow(function);
    function
        .instruction(&Instruction::LocalGet(lower))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(shorter))
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(shifted))
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Shl);
    narrow(function);
    function.instruction(&Instruction::LocalSet(product));
    if let Some(mul192) = mul192 {
        emit_mulhi_f64(function, product, mul192);
    } else {
        emit_mulhi_f32(
            function,
            product,
            mul128.expect("f32 decimal conversion needs mul128"),
        );
    }
    function
        .instruction(&Instruction::LocalTee(scaled))
        .instruction(&Instruction::I64Const(2))
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::LocalTee(below))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(above))
        .instruction(&Instruction::LocalGet(scaled))
        .instruction(&Instruction::LocalGet(below))
        .instruction(&Instruction::LocalGet(above))
        .instruction(&Instruction::I64Add);
    narrow(function);
    function
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Shl)
        .instruction(&Instruction::I64Sub);
    narrow(function);
    if mask32 {
        function
            .instruction(&Instruction::I32WrapI64)
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::I32LtS);
    } else {
        function
            .instruction(&Instruction::I64Const(0))
            .instruction(&Instruction::I64LtS);
    }
    function
        .instruction(&Instruction::LocalGet(scaled))
        .instruction(&Instruction::LocalGet(below))
        .instruction(&Instruction::LocalGet(above))
        .instruction(&Instruction::I64Add);
    narrow(function);
    function
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Shl)
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::LocalGet(below))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalSet(below_closer))
        .instruction(&Instruction::LocalGet(below))
        .instruction(&Instruction::I64Const(2))
        .instruction(&Instruction::I64Shl);
    narrow(function);
    function
        .instruction(&Instruction::LocalGet(lower))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::LocalSet(below_in))
        .instruction(&Instruction::LocalGet(below_closer))
        .instruction(&Instruction::LocalGet(below_in))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::LocalGet(below))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(above))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::End);
}

pub(super) fn compile_decimal_f32(pow10_base: i32, mul128: u32) -> Function {
    // Params are significand, binary exponent, and the regular-boundary flag.
    // The temporary product local at index 15 is shared by f32 mulhi emission.
    let mut function = Function::new([(5, ValType::I32), (13, ValType::I64)]);
    emit_decimal_prelude(&mut function, pow10_base, false);
    emit_decimal_finish(&mut function, true, Some(mul128), None);
    function
}

pub(super) fn compile_decimal_f64(pow10_base: i32, mul192: u32) -> Function {
    let mut function = Function::new([(5, ValType::I32), (13, ValType::I64)]);
    emit_decimal_prelude(&mut function, pow10_base, true);
    emit_decimal_finish(&mut function, false, None, Some(mul192));
    function
}

fn emit_store_byte(function: &mut Function, base: i32, index: u32, byte: i32) {
    function
        .instruction(&Instruction::I32Const(base))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(byte))
        .instruction(&Instruction::I32Store8(memarg(0)));
}

fn emit_literal_return(
    function: &mut Function,
    value: &[u8],
    negative: bool,
    scratch: ScratchRegion,
    string_from_memory: u32,
) {
    let index = 9;
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index));
    if negative {
        emit_store_byte(function, scratch.start(), index, b'-' as i32);
        function
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::LocalSet(index));
    }
    for byte in value {
        emit_store_byte(function, scratch.start(), index, i32::from(*byte));
        function
            .instruction(&Instruction::LocalGet(index))
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::LocalSet(index));
    }
    function
        .instruction(&Instruction::I32Const(scratch.start()))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::Call(string_from_memory))
        .instruction(&Instruction::Return);
}

fn emit_copy_digits(function: &mut Function, scratch: ScratchRegion, count: u32) {
    let output_index = 9;
    let copy_index = 10;
    let limit = 14;
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(copy_index))
        .instruction(&Instruction::LocalGet(count))
        .instruction(&Instruction::LocalSet(limit))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::LocalGet(limit))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(scratch.start()))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(scratch.at(32)))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg(0)))
        .instruction(&Instruction::I32Store8(memarg(0)))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(copy_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
}

fn emit_output_byte(function: &mut Function, scratch: ScratchRegion, byte: i32) {
    let output_index = 9;
    emit_store_byte(function, scratch.start(), output_index, byte);
    function
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index));
}

fn emit_zeros(function: &mut Function, scratch: ScratchRegion, count: u32) {
    let copy_index = 10;
    function
        .instruction(&Instruction::LocalGet(count))
        .instruction(&Instruction::LocalSet(copy_index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1));
    emit_output_byte(function, scratch, b'0' as i32);
    function
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(copy_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
}

fn compile_format_float(
    is_f32: bool,
    decimal: u32,
    string_from_memory: u32,
    scratch: ScratchRegion,
    gc: &GcLayout,
) -> Function {
    debug_assert!(scratch.capacity() >= 64);
    let string = gc.val_type(Type::Standard(StdlibTypeId::String));
    let mut function = Function::new([(14, ValType::I32), (4, ValType::I64), (1, string)]);
    let value = 0;
    let negative = 1;
    let raw_exp = 2;
    let bin_exp = 3;
    let regular = 4;
    let dec_exp = 5;
    let extra = 6;
    let digit_len = 7;
    let digit_index = 8;
    let output_index = 9;
    let copy_index = 10;
    let zero_count = 12;
    let exp_abs = 13;
    let limit = 14;
    let bits = 15;
    let bin_sig = 16;
    let dec_sig = 17;
    let remaining = 18;
    let (sig_bits, exp_bits, exp_mask, exp_offset, implicit, threshold, digits10) = if is_f32 {
        (23, 8, 0xff, 150, 1u64 << 23, 100_000_000u64, 9)
    } else {
        (
            52,
            11,
            0x7ff,
            1075,
            1u64 << 52,
            10_000_000_000_000_000u64,
            17,
        )
    };
    if is_f32 {
        function
            .instruction(&Instruction::LocalGet(value))
            .instruction(&Instruction::I32ReinterpretF32)
            .instruction(&Instruction::I64ExtendI32U)
            .instruction(&Instruction::LocalSet(bits));
    } else {
        function
            .instruction(&Instruction::LocalGet(value))
            .instruction(&Instruction::I64ReinterpretF64)
            .instruction(&Instruction::LocalSet(bits));
    }
    function
        .instruction(&Instruction::LocalGet(bits))
        .instruction(&Instruction::I64Const((sig_bits + exp_bits) as i64))
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::LocalSet(negative))
        .instruction(&Instruction::LocalGet(bits))
        .instruction(&Instruction::I64Const(sig_bits as i64))
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::I64Const(exp_mask))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::LocalSet(raw_exp))
        .instruction(&Instruction::LocalGet(bits))
        .instruction(&Instruction::I64Const((implicit - 1) as i64))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::LocalSet(bin_sig))
        .instruction(&Instruction::LocalGet(raw_exp))
        .instruction(&Instruction::I32Const(exp_mask as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(bin_sig))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    // Keep zmij's spellings for non-finite values.
    function
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::If(BlockType::Empty));
    emit_literal_return(&mut function, b"inf", true, scratch, string_from_memory);
    function.instruction(&Instruction::End);
    emit_literal_return(&mut function, b"inf", false, scratch, string_from_memory);
    function.instruction(&Instruction::Else);
    emit_literal_return(&mut function, b"NaN", false, scratch, string_from_memory);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(raw_exp))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::LocalGet(bin_sig))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::If(BlockType::Empty));
    emit_literal_return(&mut function, b"0.0", true, scratch, string_from_memory);
    function.instruction(&Instruction::End);
    emit_literal_return(&mut function, b"0.0", false, scratch, string_from_memory);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(raw_exp))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(1 - exp_offset))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(raw_exp))
        .instruction(&Instruction::I32Const(exp_offset))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(bin_exp))
        .instruction(&Instruction::LocalGet(raw_exp))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::LocalGet(bin_sig))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalSet(regular))
        .instruction(&Instruction::LocalGet(raw_exp))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(bin_sig))
        .instruction(&Instruction::I64Const(implicit as i64))
        .instruction(&Instruction::I64Or)
        .instruction(&Instruction::LocalSet(bin_sig))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(bin_sig))
        .instruction(&Instruction::LocalGet(bin_exp))
        .instruction(&Instruction::LocalGet(regular))
        .instruction(&Instruction::Call(decimal))
        .instruction(&Instruction::LocalSet(dec_exp))
        .instruction(&Instruction::LocalSet(dec_sig))
        // Subnormals may initially have fewer than the fixed output digits.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(dec_sig))
        .instruction(&Instruction::I64Const(threshold as i64))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(dec_sig))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::LocalSet(dec_sig))
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(dec_exp))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(dec_sig))
        .instruction(&Instruction::I64Const(threshold as i64))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::LocalSet(extra))
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::I32Const(digits10 - 2))
        .instruction(&Instruction::LocalGet(extra))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(dec_exp));
    if is_f32 {
        function
            .instruction(&Instruction::LocalGet(dec_sig))
            .instruction(&Instruction::I64Const(10_000_000))
            .instruction(&Instruction::I64LtU)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::LocalGet(dec_sig))
            .instruction(&Instruction::I64Const(10))
            .instruction(&Instruction::I64Mul)
            .instruction(&Instruction::LocalSet(dec_sig))
            .instruction(&Instruction::LocalGet(dec_exp))
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::I32Sub)
            .instruction(&Instruction::LocalSet(dec_exp))
            .instruction(&Instruction::End);
    }
    // Trim insignificant trailing zeros, then materialize the remaining digits
    // in the second half of the scratch region.
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(dec_sig))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64RemU)
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(dec_sig))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64DivU)
        .instruction(&Instruction::LocalSet(dec_sig))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(dec_sig))
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(digit_len))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(digit_len))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalTee(digit_len))
        .instruction(&Instruction::LocalSet(digit_index))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64DivU)
        .instruction(&Instruction::LocalTee(remaining))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(dec_sig))
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::LocalGet(digit_len))
        .instruction(&Instruction::LocalSet(digit_index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(digit_index))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(digit_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalTee(digit_index))
        .instruction(&Instruction::I32Const(scratch.at(32)))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64RemU)
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Store8(memarg(0)))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64DivU)
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::If(BlockType::Empty));
    emit_output_byte(&mut function, scratch, b'-' as i32);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::I32Const(if is_f32 { -6 } else { -5 }))
        .instruction(&Instruction::I32GeS)
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::I32Const(if is_f32 { 12 } else { 15 }))
        .instruction(&Instruction::I32LeS)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(digit_len))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::I32LeS)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_copy_digits(&mut function, scratch, digit_len);
    function
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(digit_len))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(zero_count));
    emit_zeros(&mut function, scratch, zero_count);
    emit_output_byte(&mut function, scratch, b'.' as i32);
    emit_output_byte(&mut function, scratch, b'0' as i32);
    function
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32GeS)
        .instruction(&Instruction::If(BlockType::Empty))
        // Copy the prefix, insert '.', then copy the remaining suffix.
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(limit));
    emit_copy_digits(&mut function, scratch, limit);
    emit_output_byte(&mut function, scratch, b'.' as i32);
    // Move the digit scratch pointer logically by copying the suffix manually.
    function
        .instruction(&Instruction::LocalGet(limit))
        .instruction(&Instruction::LocalSet(copy_index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::LocalGet(digit_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(scratch.start()))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(scratch.at(32)))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg(0)))
        .instruction(&Instruction::I32Store8(memarg(0)))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(copy_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::Else);
    emit_output_byte(&mut function, scratch, b'0' as i32);
    emit_output_byte(&mut function, scratch, b'.' as i32);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(zero_count));
    emit_zeros(&mut function, scratch, zero_count);
    emit_copy_digits(&mut function, scratch, digit_len);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::Else);
    // Scientific notation.
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(limit));
    emit_copy_digits(&mut function, scratch, limit);
    function
        .instruction(&Instruction::LocalGet(digit_len))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_output_byte(&mut function, scratch, b'.' as i32);
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(copy_index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::LocalGet(digit_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(scratch.start()))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(scratch.at(32)))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg(0)))
        .instruction(&Instruction::I32Store8(memarg(0)))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(copy_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
    emit_output_byte(&mut function, scratch, b'e' as i32);
    function
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32GeS)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(b'+' as i32))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(b'-' as i32))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(limit));
    emit_output_byte(&mut function, scratch, 0);
    // The previous helper emitted a literal zero; replace it with the sign.
    function
        .instruction(&Instruction::I32Const(scratch.start()))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(limit))
        .instruction(&Instruction::I32Store8(memarg(0)))
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32LtS)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(dec_exp))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(exp_abs))
        .instruction(&Instruction::LocalGet(exp_abs))
        .instruction(&Instruction::I32Const(100))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(exp_abs))
        .instruction(&Instruction::I32Const(100))
        .instruction(&Instruction::I32DivU)
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(limit));
    emit_output_byte(&mut function, scratch, 0);
    function
        .instruction(&Instruction::I32Const(scratch.start()))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(limit))
        .instruction(&Instruction::I32Store8(memarg(0)))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(exp_abs))
        .instruction(&Instruction::I32Const(10))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(exp_abs))
        .instruction(&Instruction::I32Const(100))
        .instruction(&Instruction::I32RemU)
        .instruction(&Instruction::I32Const(10))
        .instruction(&Instruction::I32DivU)
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(limit));
    emit_output_byte(&mut function, scratch, 0);
    function
        .instruction(&Instruction::I32Const(scratch.start()))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(limit))
        .instruction(&Instruction::I32Store8(memarg(0)))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(exp_abs))
        .instruction(&Instruction::I32Const(10))
        .instruction(&Instruction::I32RemU)
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(limit));
    emit_output_byte(&mut function, scratch, 0);
    function
        .instruction(&Instruction::I32Const(scratch.start()))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(limit))
        .instruction(&Instruction::I32Store8(memarg(0)))
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(scratch.start()))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::Call(string_from_memory))
        .instruction(&Instruction::End);
    function
}

pub(super) fn compile_format_f32(
    decimal: u32,
    string_from_memory: u32,
    scratch: ScratchRegion,
    gc: &GcLayout,
) -> Function {
    compile_format_float(true, decimal, string_from_memory, scratch, gc)
}

pub(super) fn compile_format_f64(
    decimal: u32,
    string_from_memory: u32,
    scratch: ScratchRegion,
    gc: &GcLayout,
) -> Function {
    compile_format_float(false, decimal, string_from_memory, scratch, gc)
}

#[cfg(test)]
mod tests {
    use super::{POW10_BYTES, pow10_significands_bytes};

    #[test]
    fn zmij_power_table_has_the_expected_shape() {
        let bytes = pow10_significands_bytes();
        assert_eq!(bytes.len(), POW10_BYTES);
        assert!(bytes.iter().any(|byte| *byte != 0));
    }
}
