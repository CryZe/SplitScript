//! Repeatable local compile-time and generated-Wasm-size baseline.
//!
//! Run with `cargo run --release --example compiler_baseline -- 200`.
//! Append `--frontend` to measure parsing, library augmentation, and declaration
//! resolution without type checking or Wasm generation (including result drop).

use std::{hint::black_box, time::Instant};

const DEFAULT_ITERATIONS: usize = 200;
const WARMUP_ITERATIONS: usize = 20;

const FIXTURES: [(&str, &str); 4] = [
    ("minimal", "state \"game.exe\" {}"),
    ("lunistice", include_str!("lunistice.split")),
    ("cancellation", include_str!("cancellation.split")),
    ("settings", include_str!("lso_desktop_settings.split")),
];

fn main() {
    let mut arguments = std::env::args().skip(1);
    let iterations = arguments
        .next()
        .map(|value| {
            value
                .parse::<usize>()
                .expect("iteration count must be a positive integer")
        })
        .unwrap_or(DEFAULT_ITERATIONS);
    assert!(iterations > 0, "iteration count must be positive");
    let frontend = match arguments.next().as_deref() {
        None => false,
        Some("--frontend") => true,
        Some(other) => panic!("unknown baseline option: {other}"),
    };
    assert!(arguments.next().is_none(), "too many baseline arguments");

    println!("rust_harness_profile=release");
    println!(
        "splitscript_profile={}",
        if frontend { "n/a" } else { "release" }
    );
    println!("pipeline={}", if frontend { "frontend" } else { "compile" });
    println!(
        "platform={}-{} logical_cpus={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(1, usize::from)
    );
    println!("warmup_iterations={WARMUP_ITERATIONS} measured_iterations={iterations}");
    println!("fixture\tsource_bytes\twasm_bytes\tmedian_us\tp95_us");

    for (name, source) in FIXTURES {
        for _ in 0..WARMUP_ITERATIONS {
            black_box(run(source, frontend));
        }

        let mut samples = Vec::with_capacity(iterations);
        let mut wasm_bytes = 0;
        for _ in 0..iterations {
            let start = Instant::now();
            let wasm = run(black_box(source), frontend);
            samples.push(start.elapsed().as_nanos());
            wasm_bytes = wasm.len();
            black_box(wasm);
        }
        samples.sort_unstable();
        let median = samples[samples.len() / 2];
        let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
        println!(
            "{name}\t{}\t{}\t{}\t{}",
            source.len(),
            if frontend {
                "-".to_owned()
            } else {
                wasm_bytes.to_string()
            },
            nanos_to_micros(median),
            nanos_to_micros(p95)
        );
    }
}

fn run(source: &str, frontend: bool) -> Vec<u8> {
    if frontend {
        black_box(splitscript::lower(
            splitscript::parse(source).expect("baseline fixture should parse"),
        ));
        return Vec::new();
    }
    splitscript::compile_with_options(
        source,
        splitscript::CompilerOptions {
            profile: splitscript::BuildProfile::Release,
            ..splitscript::CompilerOptions::default()
        },
    )
    .unwrap_or_else(|diagnostics| {
        panic!(
            "baseline fixture failed to compile: {}",
            diagnostics
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )
    })
}

fn nanos_to_micros(nanos: u128) -> String {
    format!("{:.1}", nanos as f64 / 1_000.0)
}
