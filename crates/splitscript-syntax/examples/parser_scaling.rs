//! Manual parser scaling baseline: `cargo run -p splitscript-syntax --release
//! --example parser_scaling -- 50` (also supports `--profile max-opt`).
//! Token cloning and lexing are outside the timer; syntax/token disposal is inside.

use std::{fmt::Write, hint::black_box, time::Instant};

use splitscript_syntax::{SyntaxMode, lex, parser};

fn main() {
    let iterations = std::env::args()
        .nth(1)
        .map(|value| value.parse::<usize>().expect("positive iteration count"))
        .unwrap_or(50);
    assert!(iterations > 0);
    println!("warmup_iterations=10 measured_iterations={iterations}");
    println!("functions\tsource_bytes\ttokens\tmedian_us\tp95_us");
    for functions in [100, 500, 1_000, 2_000, 4_000] {
        let mut source = String::from("state \"game.exe\" {}\n");
        for index in 0..functions {
            writeln!(
                source,
                "fn helper{index}() {{ if true {{ return 1 }} return 0 }}"
            )
            .unwrap();
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
