//! Manual parser scaling baseline: `cargo run -p splitscript-syntax --release
//! --example parser_scaling -- 50` (also supports `--profile max-opt`).
//! Token cloning and lexing are outside the timer; syntax/token disposal is inside.
//! Append `--calls` to exercise nested calls and array argument lists.

use std::{fmt::Write, hint::black_box, time::Instant};

use splitscript_syntax::{SyntaxMode, lex, parser};

fn main() {
    let mut arguments = std::env::args().skip(1);
    let iterations = arguments
        .next()
        .map(|value| value.parse::<usize>().expect("positive iteration count"))
        .unwrap_or(50);
    assert!(iterations > 0);
    let calls = match arguments.next().as_deref() {
        None => false,
        Some("--calls") => true,
        Some(other) => panic!("unknown benchmark option: {other}"),
    };
    assert!(arguments.next().is_none(), "too many benchmark arguments");
    println!("fixture={}", if calls { "calls" } else { "blocks" });
    println!("warmup_iterations=10 measured_iterations={iterations}");
    println!("functions\tsource_bytes\ttokens\tmedian_us\tp95_us");
    for functions in [100, 500, 1_000, 2_000, 4_000] {
        let mut source = String::from("state \"game.exe\" {}\n");
        for index in 0..functions {
            if calls {
                writeln!(
                    source,
                    "fn helper{index}() {{ return consume([1, 2], pair(3, 4)) }}"
                )
                .unwrap();
            } else {
                writeln!(
                    source,
                    "fn helper{index}() {{ if true {{ return 1 }} return 0 }}"
                )
                .unwrap();
            }
        }
        let tokens = lex(&source, SyntaxMode::Program).expect("valid fixture tokens");
        let mut samples = Vec::with_capacity(iterations);
        for iteration in 0..10 + iterations {
            let owned_tokens = tokens.clone();
            let start = Instant::now();
            let parsed = parser::parse(black_box(&source), owned_tokens).expect("valid fixture");
            assert_eq!(parsed.functions.len(), functions);
            drop(black_box(parsed));
            if iteration >= 10 {
                samples.push(start.elapsed().as_nanos());
            }
        }
        samples.sort_unstable();
        println!(
            "{functions}\t{}\t{}\t{:.1}\t{:.1}",
            source.len(),
            tokens.len(),
            samples[samples.len() / 2] as f64 / 1_000.0,
            samples[(samples.len() * 95).div_ceil(100) - 1] as f64 / 1_000.0,
        );
    }
}
