//! Repeatable local compile-time and generated-Wasm-size baseline.
//!
//! Run with `cargo run --release --example compiler_baseline -- 200`.

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
    let iterations = std::env::args()
        .nth(1)
        .map(|value| {
            value
                .parse::<usize>()
                .expect("iteration count must be a positive integer")
        })
        .unwrap_or(DEFAULT_ITERATIONS);
    assert!(iterations > 0, "iteration count must be positive");

    println!("profile=release");
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
            black_box(compile(source));
        }

        let mut samples = Vec::with_capacity(iterations);
        let mut wasm_bytes = 0;
        for _ in 0..iterations {
            let start = Instant::now();
            let wasm = compile(black_box(source));
            samples.push(start.elapsed().as_nanos());
            wasm_bytes = wasm.len();
            black_box(wasm);
        }
        samples.sort_unstable();
        let median = samples[samples.len() / 2];
        let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
        println!(
            "{name}\t{}\t{wasm_bytes}\t{}\t{}",
            source.len(),
            nanos_to_micros(median),
            nanos_to_micros(p95)
        );
    }
}

fn compile(source: &str) -> Vec<u8> {
    splitscript::compile(source).unwrap_or_else(|diagnostics| {
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
