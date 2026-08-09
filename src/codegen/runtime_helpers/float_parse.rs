//! Correctly rounded decimal-to-binary floating-point conversion.
//!
//! This is an allocation-free WebAssembly implementation of the "Simple
//! Decimal Conversion" algorithm by Nigel Tao and Ken Thompson. The same
//! algorithm is the exact fallback in Rust's `core::num::dec2flt`: decimal
//! digits are shifted into the binary significand range and rounded once,
//! ties to even. A 768-digit buffer is sufficient to disambiguate every
//! binary64 result, including subnormals and adversarial halfway inputs.

use wasm_encoder::{BlockType, Function, Instruction, MemArg, ValType};

use crate::codegen::memory_plan::{FLOAT_PARSE_DIGITS, ScratchRegion};
use crate::stdlib::StdlibTypeId;

use super::super::GcLayout;

const MAX_DIGITS: i32 = FLOAT_PARSE_DIGITS as i32;
const DECIMAL_POINT_RANGE: i32 = 2047;
const MAX_SHIFT: i32 = 60;

fn memarg() -> MemArg {
    MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }
}

fn emit_digit_address(function: &mut Function, region: ScratchRegion, index: u32) {
    function
        .instruction(&Instruction::I32Const(region.start()))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add);
}

fn emit_digit_store_from_local(
    function: &mut Function,
    region: ScratchRegion,
    index: u32,
    value: u32,
) {
    emit_digit_address(function, region, index);
    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::I32Store8(memarg()));
}

/// Parses the complete ASCII spelling and converts it exactly once to the
/// requested IEEE-754 width. The status result is false only for invalid
/// grammar; finite overflow is a successful infinity, matching Rust-style
/// floating-point parsing.
pub(super) fn compile_string_parse_float(
    left_shift: u32,
    right_shift: u32,
    round: u32,
    digits: ScratchRegion,
    gc: &GcLayout,
) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([(20, ValType::I32), (2, ValType::I64)]);
    let value = 0;
    let target_is_f32 = 1;
    let len = 2;
    let index = 3;
    let byte = 4;
    let negative = 5;
    let saw_digit = 6;
    let saw_dot = 7;
    let digits_before_dot = 8;
    let total_digits = 9;
    let first_nonzero = 10;
    let last_nonzero = 11;
    let num_digits = 12;
    let decimal_point = 13;
    let truncated = 14;
    let exponent_negative = 15;
    let exponent_value = 16;
    let exp2 = 17;
    let shift = 18;
    let significand_bits = 19;
    let exponent_min = 20;
    let infinite_power = 21;
    let mantissa = 22;
    let word = 23;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalTee(len))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_float_parse_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::LocalSet(first_nonzero))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::LocalSet(last_nonzero))
        // Consume one optional sign.
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalSet(byte))
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'+' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'-' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(negative))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_float_parse_failure(&mut function);
    function.instruction(&Instruction::End);

    // Rust-compatible, case-insensitive non-finite spellings.
    emit_tail_eq_ignore_ascii_case(&mut function, value, index, len, string_type, b"nan");
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_special_float(&mut function, negative, f64::NAN);
    function.instruction(&Instruction::End);
    for spelling in [b"inf".as_slice(), b"infinity".as_slice()] {
        emit_tail_eq_ignore_ascii_case(&mut function, value, index, len, string_type, spelling);
        function.instruction(&Instruction::If(BlockType::Empty));
        emit_special_float(&mut function, negative, f64::INFINITY);
        function.instruction(&Instruction::End);
    }

    // Parse the mantissa while retaining only significant decimal digits.
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalSet(byte))
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'9' as i32))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(saw_digit))
        .instruction(&Instruction::LocalGet(saw_dot))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(digits_before_dot))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(digits_before_dot))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(first_nonzero))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(total_digits))
        .instruction(&Instruction::LocalSet(first_nonzero))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(first_nonzero))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(total_digits))
        .instruction(&Instruction::LocalGet(first_nonzero))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(shift))
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::I32Const(MAX_DIGITS))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(exp2));
    emit_digit_store_from_local(&mut function, digits, shift, exp2);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::LocalSet(last_nonzero))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(total_digits))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(total_digits))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(1))
        .instruction(&Instruction::End)
        // One decimal point is allowed, with digits on either side in total.
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'.' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(saw_dot))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(saw_dot))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(1))
        .instruction(&Instruction::End)
        // Leave an exponent marker for the exponent parser.
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'e' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'E' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::BrIf(1));
    emit_float_parse_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(saw_digit))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_float_parse_failure(&mut function);
    function.instruction(&Instruction::End);

    // Parse the optional exponent, saturating only after it is already far
    // beyond either IEEE format's meaningful range.
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_float_parse_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalSet(byte))
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'+' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'-' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(exponent_negative))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(saw_digit))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalTee(byte))
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'9' as i32))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_float_parse_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(saw_digit))
        .instruction(&Instruction::LocalGet(exponent_value))
        .instruction(&Instruction::I32Const(100_000))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(exponent_value))
        .instruction(&Instruction::I32Const(10))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalTee(exponent_value))
        .instruction(&Instruction::I32Const(100_000))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(100_000))
        .instruction(&Instruction::LocalSet(exponent_value))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(saw_digit))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_float_parse_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);

    // All-zero numeric spellings preserve their sign without running the
    // decimal conversion machinery.
    function
        .instruction(&Instruction::LocalGet(first_nonzero))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_special_float(&mut function, negative, 0.0);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(last_nonzero))
        .instruction(&Instruction::I32Const(MAX_DIGITS))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(MAX_DIGITS))
        .instruction(&Instruction::LocalSet(num_digits))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(truncated))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(last_nonzero))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(num_digits))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(digits_before_dot))
        .instruction(&Instruction::LocalGet(first_nonzero))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(decimal_point))
        .instruction(&Instruction::LocalGet(exponent_negative))
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(exponent_value))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(exponent_value))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(decimal_point))
        // Width-specific IEEE parameters.
        .instruction(&Instruction::LocalGet(target_is_f32))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(23))
        .instruction(&Instruction::LocalSet(significand_bits))
        .instruction(&Instruction::I32Const(-126))
        .instruction(&Instruction::LocalSet(exponent_min))
        .instruction(&Instruction::I32Const(255))
        .instruction(&Instruction::LocalSet(infinite_power))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(52))
        .instruction(&Instruction::LocalSet(significand_bits))
        .instruction(&Instruction::I32Const(-1022))
        .instruction(&Instruction::LocalSet(exponent_min))
        .instruction(&Instruction::I32Const(2047))
        .instruction(&Instruction::LocalSet(infinite_power))
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(exp2))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Const(-324))
        .instruction(&Instruction::I32LtS)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_special_float(&mut function, negative, 0.0);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Const(310))
        .instruction(&Instruction::I32GeS)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_special_float(&mut function, negative, f64::INFINITY);
    function.instruction(&Instruction::End);

    // Shift right toward (1/2, 1].
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32LeS)
        .instruction(&Instruction::BrIf(1));
    emit_get_shift(&mut function, decimal_point);
    function
        .instruction(&Instruction::LocalSet(shift))
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::LocalGet(truncated))
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::Call(right_shift));
    emit_store_decimal_result(&mut function, num_digits, decimal_point, truncated);
    function
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_special_float(&mut function, negative, 0.0);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(exp2))
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(exp2))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        // Shift left toward (1/2, 1].
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32GtS)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_first_digit_load(&mut function, digits);
    function
        .instruction(&Instruction::I32Const(5))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(2))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    emit_first_digit_load(&mut function, digits);
    function
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(shift));
    emit_get_shift(&mut function, shift);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(shift))
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::LocalGet(truncated))
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::Call(left_shift));
    emit_store_decimal_result(&mut function, num_digits, decimal_point, truncated);
    function
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Const(DECIMAL_POINT_RANGE))
        .instruction(&Instruction::I32GtS)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_special_float(&mut function, negative, f64::INFINITY);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(exp2))
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(exp2))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(exp2))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(exp2));

    // Bring subnormals to the minimum exponent before significand rounding.
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(exponent_min))
        .instruction(&Instruction::LocalGet(exp2))
        .instruction(&Instruction::I32LeS)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(exponent_min))
        .instruction(&Instruction::LocalGet(exp2))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalTee(shift))
        .instruction(&Instruction::I32Const(MAX_SHIFT))
        .instruction(&Instruction::I32GtS)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(MAX_SHIFT))
        .instruction(&Instruction::LocalSet(shift))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::LocalGet(truncated))
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::Call(right_shift));
    emit_store_decimal_result(&mut function, num_digits, decimal_point, truncated);
    function
        .instruction(&Instruction::LocalGet(exp2))
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(exp2))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(exp2))
        .instruction(&Instruction::LocalGet(exponent_min))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(infinite_power))
        .instruction(&Instruction::I32GeS)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_special_float(&mut function, negative, f64::INFINITY);
    function
        .instruction(&Instruction::End)
        // Shift the hidden bit into place, then round exactly once.
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::LocalGet(truncated))
        .instruction(&Instruction::LocalGet(significand_bits))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::Call(left_shift));
    emit_store_decimal_result(&mut function, num_digits, decimal_point, truncated);
    function
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::LocalGet(truncated))
        .instruction(&Instruction::Call(round))
        .instruction(&Instruction::LocalSet(mantissa))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::LocalGet(significand_bits))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Shl)
        .instruction(&Instruction::LocalGet(mantissa))
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::LocalGet(truncated))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Call(right_shift));
    emit_store_decimal_result(&mut function, num_digits, decimal_point, truncated);
    function
        .instruction(&Instruction::LocalGet(exp2))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(exp2))
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::LocalGet(truncated))
        .instruction(&Instruction::Call(round))
        .instruction(&Instruction::LocalSet(mantissa))
        .instruction(&Instruction::LocalGet(exp2))
        .instruction(&Instruction::LocalGet(exponent_min))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(infinite_power))
        .instruction(&Instruction::I32GeS)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_special_float(&mut function, negative, f64::INFINITY);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(exp2))
        .instruction(&Instruction::LocalGet(exponent_min))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(shift))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::LocalGet(significand_bits))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Shl)
        .instruction(&Instruction::LocalGet(mantissa))
        .instruction(&Instruction::I64GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(shift))
        .instruction(&Instruction::End)
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::LocalGet(significand_bits))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Shl)
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalGet(mantissa))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalGet(significand_bits))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Shl)
        .instruction(&Instruction::I64Or)
        .instruction(&Instruction::LocalSet(word))
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(word))
        .instruction(&Instruction::LocalGet(target_is_f32))
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I64Const(1_i64 << 31))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I64Const(i64::MIN))
        .instruction(&Instruction::End)
        .instruction(&Instruction::I64Or)
        .instruction(&Instruction::LocalSet(word))
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalGet(target_is_f32))
        .instruction(&Instruction::If(BlockType::Result(ValType::F64)))
        .instruction(&Instruction::LocalGet(word))
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::F32ReinterpretI32)
        .instruction(&Instruction::F64PromoteF32)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(word))
        .instruction(&Instruction::F64ReinterpretI64)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
    function
}

fn emit_store_decimal_result(
    function: &mut Function,
    num_digits: u32,
    decimal_point: u32,
    truncated: u32,
) {
    function
        .instruction(&Instruction::LocalSet(truncated))
        .instruction(&Instruction::LocalSet(decimal_point))
        .instruction(&Instruction::LocalSet(num_digits));
}

fn emit_get_shift(function: &mut Function, value: u32) {
    const SHIFTS: [i32; 19] = [
        0, 3, 6, 9, 13, 16, 19, 23, 26, 29, 33, 36, 39, 43, 46, 49, 53, 56, 59,
    ];
    for (input, output) in SHIFTS.into_iter().enumerate().skip(1) {
        function
            .instruction(&Instruction::LocalGet(value))
            .instruction(&Instruction::I32Const(input as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
            .instruction(&Instruction::I32Const(output))
            .instruction(&Instruction::Else);
    }
    function.instruction(&Instruction::I32Const(MAX_SHIFT));
    for _ in 1..SHIFTS.len() {
        function.instruction(&Instruction::End);
    }
}

fn emit_tail_eq_ignore_ascii_case(
    function: &mut Function,
    value: u32,
    start: u32,
    len: u32,
    string_type: u32,
    expected: &[u8],
) {
    function
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Const(expected.len() as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(1));
    for (offset, expected) in expected.iter().copied().enumerate() {
        function
            .instruction(&Instruction::LocalGet(value))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::LocalGet(start))
            .instruction(&Instruction::I32Const(offset as i32))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::ArrayGetU(string_type))
            .instruction(&Instruction::I32Const(0x20))
            .instruction(&Instruction::I32Or)
            .instruction(&Instruction::I32Const(i32::from(expected)))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::I32And);
    }
    function
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::End);
}

fn emit_first_digit_load(function: &mut Function, digits: ScratchRegion) {
    function
        .instruction(&Instruction::I32Const(digits.start()))
        .instruction(&Instruction::I32Load8U(memarg()));
}

fn emit_special_float(function: &mut Function, negative: u32, value: f64) {
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::If(BlockType::Result(ValType::F64)))
        .instruction(&Instruction::F64Const(value.into()))
        .instruction(&Instruction::F64Neg)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::F64Const(value.into()))
        .instruction(&Instruction::End)
        .instruction(&Instruction::Return);
}

fn emit_float_parse_failure(function: &mut Function) {
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::F64Const(0.0.into()))
        .instruction(&Instruction::Return);
}
