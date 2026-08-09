//! Exact fixed-buffer decimal shifting and ties-to-even rounding.
//!
//! These helpers implement the allocation-free "Simple Decimal Conversion"
//! operations used by the floating-point string parser.

use wasm_encoder::{BlockType, Function, Instruction, MemArg, ValType};

use crate::codegen::memory_plan::{FLOAT_PARSE_DIGITS, ScratchRegion};

const MAX_DIGITS: i32 = FLOAT_PARSE_DIGITS as i32;
const DECIMAL_POINT_RANGE: i32 = 2047;

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

fn emit_digit_load(function: &mut Function, region: ScratchRegion, index: u32) {
    emit_digit_address(function, region, index);
    function.instruction(&Instruction::I32Load8U(memarg()));
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

fn emit_trim(function: &mut Function, digits: ScratchRegion, num_digits: u32) {
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(num_digits));
    emit_digit_load(function, digits, num_digits);
    function
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(num_digits))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
}

/// Computes `decimal * 2^shift` through an 800-byte companion buffer. Keeping
/// the complete (at most 787-digit) product until trailing-zero trimming means
/// truncation is recorded only when discarded digits carry information.
pub(super) fn compile_decimal_left_shift(
    digits: ScratchRegion,
    temporary: ScratchRegion,
) -> Function {
    debug_assert!(digits.capacity() >= MAX_DIGITS);
    debug_assert!(temporary.capacity() >= 800);
    let mut function = Function::new([(8, ValType::I32), (4, ValType::I64)]);
    let num_digits = 0;
    let decimal_point = 1;
    let truncated = 2;
    let shift = 3;
    let original_num_digits = 4;
    let read_index = 5;
    let write_index = 6;
    let raw_output_len = 7;
    let output_len = 8;
    let copy_index = 9;
    let source_index = 10;
    let digit_i32 = 11;
    let carry = 12;
    let n = 13;
    let quotient = 14;
    let remainder = 15;

    function
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::LocalGet(truncated))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::LocalSet(original_num_digits))
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::LocalSet(read_index))
        .instruction(&Instruction::I32Const(800))
        .instruction(&Instruction::LocalSet(write_index))
        // Multiply from least to most significant digit.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(read_index))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(read_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalTee(read_index));
    emit_digit_load(&mut function, digits, read_index);
    function
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Shl)
        .instruction(&Instruction::LocalGet(carry))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalTee(n))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64DivU)
        .instruction(&Instruction::LocalTee(quotient))
        .instruction(&Instruction::LocalSet(carry))
        .instruction(&Instruction::LocalGet(n))
        .instruction(&Instruction::LocalGet(quotient))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalSet(remainder))
        .instruction(&Instruction::LocalGet(write_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalTee(write_index))
        .instruction(&Instruction::LocalSet(source_index))
        .instruction(&Instruction::LocalGet(remainder))
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::LocalSet(digit_i32));
    emit_digit_store_from_local(&mut function, temporary, source_index, digit_i32);
    function
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        // Emit every remaining carry digit.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(carry))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(carry))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64DivU)
        .instruction(&Instruction::LocalSet(quotient))
        .instruction(&Instruction::LocalGet(carry))
        .instruction(&Instruction::LocalGet(quotient))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalSet(remainder))
        .instruction(&Instruction::LocalGet(quotient))
        .instruction(&Instruction::LocalSet(carry))
        .instruction(&Instruction::LocalGet(write_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalTee(write_index))
        .instruction(&Instruction::LocalSet(source_index))
        .instruction(&Instruction::LocalGet(remainder))
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::LocalSet(digit_i32));
    emit_digit_store_from_local(&mut function, temporary, source_index, digit_i32);
    function
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(800))
        .instruction(&Instruction::LocalGet(write_index))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalTee(raw_output_len))
        .instruction(&Instruction::LocalSet(output_len))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::LocalGet(raw_output_len))
        .instruction(&Instruction::LocalGet(original_num_digits))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(decimal_point))
        // Trim exact trailing decimal zeroes before deciding truncation.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(output_len))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(write_index))
        .instruction(&Instruction::LocalGet(output_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(source_index));
    emit_digit_load(&mut function, temporary, source_index);
    function
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(output_len))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(output_len))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output_len))
        .instruction(&Instruction::I32Const(MAX_DIGITS))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(truncated))
        .instruction(&Instruction::I32Const(MAX_DIGITS))
        .instruction(&Instruction::LocalSet(num_digits))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(output_len))
        .instruction(&Instruction::LocalSet(num_digits))
        .instruction(&Instruction::End)
        // Copy the most significant retained digits back to the primary bank.
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(copy_index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(write_index))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(source_index));
    emit_digit_load(&mut function, temporary, source_index);
    function.instruction(&Instruction::LocalSet(digit_i32));
    emit_digit_store_from_local(&mut function, digits, copy_index, digit_i32);
    function
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(copy_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::LocalGet(truncated))
        .instruction(&Instruction::End);
    function
}

/// Computes `decimal * 2^-shift` in place. Division by a power of two has a
/// terminating decimal expansion, so at most `shift` additional digits are
/// needed after the input has been consumed.
pub(super) fn compile_decimal_right_shift(digits: ScratchRegion) -> Function {
    debug_assert!(digits.capacity() >= MAX_DIGITS);
    let mut function = Function::new([(5, ValType::I32), (2, ValType::I64)]);
    let num_digits = 0;
    let decimal_point = 1;
    let truncated = 2;
    let shift = 3;
    let read_index = 4;
    let write_index = 5;
    let new_digit = 6;
    let source_digit = 7;
    let original_num_digits = 8;
    let mask = 9;
    let n = 10;

    function
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::LocalSet(original_num_digits))
        // Consume enough leading decimal digits for the first non-zero
        // quotient digit. Appended zeroes move the decimal point when the
        // input integer is smaller than the binary divisor.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(n))
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(read_index))
        .instruction(&Instruction::LocalGet(original_num_digits))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    emit_digit_load(&mut function, digits, read_index);
    function
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(source_digit))
        .instruction(&Instruction::LocalGet(read_index))
        .instruction(&Instruction::LocalGet(original_num_digits))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(n))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(n))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::LocalGet(source_digit))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(n))
        .instruction(&Instruction::LocalGet(read_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(read_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::LocalGet(read_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(decimal_point))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Const(-DECIMAL_POINT_RANGE))
        .instruction(&Instruction::I32LtS)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Shl)
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalSet(mask))
        // Consume the remaining original digits.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(read_index))
        .instruction(&Instruction::LocalGet(original_num_digits))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(n))
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::LocalSet(new_digit))
        .instruction(&Instruction::LocalGet(n))
        .instruction(&Instruction::LocalGet(mask))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64Mul);
    emit_digit_load(&mut function, digits, read_index);
    function
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(n));
    emit_digit_store_from_local(&mut function, digits, write_index, new_digit);
    function
        .instruction(&Instruction::LocalGet(read_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(read_index))
        .instruction(&Instruction::LocalGet(write_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(write_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        // Continue until the power-of-two remainder terminates.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(n))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(n))
        .instruction(&Instruction::LocalGet(shift))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64ShrU)
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::LocalSet(new_digit))
        .instruction(&Instruction::LocalGet(n))
        .instruction(&Instruction::LocalGet(mask))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::LocalSet(n))
        .instruction(&Instruction::LocalGet(write_index))
        .instruction(&Instruction::I32Const(MAX_DIGITS))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_digit_store_from_local(&mut function, digits, write_index, new_digit);
    function
        .instruction(&Instruction::LocalGet(write_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(write_index))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(new_digit))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(truncated))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(write_index))
        .instruction(&Instruction::LocalSet(num_digits));
    emit_trim(&mut function, digits, num_digits);
    function
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::LocalGet(truncated))
        .instruction(&Instruction::End);
    function
}

/// Rounds the decimal at its current decimal point to an unsigned integer,
/// using IEEE-754 round-to-nearest, ties-to-even and the sticky truncation bit.
pub(super) fn compile_decimal_round(digits: ScratchRegion) -> Function {
    let mut function = Function::new([(4, ValType::I32), (1, ValType::I64)]);
    let num_digits = 0;
    let decimal_point = 1;
    let truncated = 2;
    let index = 3;
    let digit = 4;
    let round_up = 5;
    let previous_digit = 6;
    let value = 7;

    function
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32LtS)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Const(19))
        .instruction(&Instruction::I32GeS)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(-1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        // Fold every integral decimal position into a u64.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    emit_digit_load(&mut function, digits, index);
    function
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(value))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        // Inspect the first discarded digit.
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_digit_load(&mut function, digits, decimal_point);
    function
        .instruction(&Instruction::LocalTee(digit))
        .instruction(&Instruction::I32Const(5))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalSet(round_up))
        .instruction(&Instruction::LocalGet(digit))
        .instruction(&Instruction::I32Const(5))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(num_digits))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(decimal_point))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(index));
    emit_digit_load(&mut function, digits, index);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(previous_digit))
        .instruction(&Instruction::LocalGet(truncated))
        .instruction(&Instruction::LocalGet(previous_digit))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalSet(round_up))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(round_up))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(value))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::End);
    function
}
