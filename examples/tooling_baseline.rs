//! Repeatable warm compiler-database and in-process LSP latency baseline.
//!
//! Run with `cargo run --release --example tooling_baseline -- 500 200`.

use std::{hint::black_box, time::Instant};

use serde_json::json;
use splitscript::tooling::{database::CompilerDatabase, lsp::LanguageServer};

const DEFAULT_FUNCTIONS: usize = 500;
const DEFAULT_ITERATIONS: usize = 200;
const WARMUP_ITERATIONS: usize = 20;

fn main() {
    let mut arguments = std::env::args().skip(1);
    let functions = parse_positive(arguments.next(), DEFAULT_FUNCTIONS, "function count");
    let iterations = parse_positive(arguments.next(), DEFAULT_ITERATIONS, "iteration count");
    assert!(arguments.next().is_none(), "expected at most two arguments");

    let source = large_source(functions);
    let target = format!("helper{}", functions - 1);
    let offset = source
        .rfind(&target)
        .expect("generated source must call its final helper");
    let (line, character) = line_character(&source, offset);

    println!("profile=release");
    println!(
        "platform={}-{} logical_cpus={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(1, usize::from)
    );
    println!(
        "functions={functions} source_bytes={} warmup_iterations={WARMUP_ITERATIONS} measured_iterations={iterations}",
        source.len()
    );
    println!("query\tmedian_us\tp95_us");

    let mut database = CompilerDatabase::new(source.clone());
    measure("database_cold_check", 1, 0, || {
        black_box(database.check().expect("large fixture must type-check"));
    });
    measure("database_warm_check", iterations, WARMUP_ITERATIONS, || {
        black_box(
            database
                .check()
                .expect("cached check must remain available"),
        );
    });
    measure("database_warm_hover", iterations, WARMUP_ITERATIONS, || {
        black_box(database.hover(offset).expect("hover query must succeed"));
    });
    measure(
        "database_warm_highlights",
        iterations,
        WARMUP_ITERATIONS,
        || {
            black_box(
                database
                    .semantic_highlights()
                    .expect("highlight query must succeed"),
            );
        },
    );
    measure(
        "database_warm_definitions",
        iterations,
        WARMUP_ITERATIONS,
        || {
            black_box(
                database
                    .definition_index()
                    .expect("definition query must succeed"),
            );
        },
    );

    let uri = "file:///tooling-baseline.split";
    let mut server = LanguageServer::default();
    black_box(server.handle(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    })));
    black_box(server.handle(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": uri,
                "languageId": "splitscript",
                "version": 1,
                "text": source
            }
        }
    })));
    let mut request_id = 2_u64;
    measure("lsp_warm_hover", iterations, WARMUP_ITERATIONS, || {
        let response = server.handle(json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }
        }));
        request_id += 1;
        assert_eq!(response.len(), 1, "hover request must produce one response");
        black_box(response);
    });
}

fn parse_positive(value: Option<String>, default: usize, label: &str) -> usize {
    let value = value.map_or(default, |value| {
        value
            .parse::<usize>()
            .unwrap_or_else(|_| panic!("{label} must be a positive integer"))
    });
    assert!(value > 0, "{label} must be positive");
    value
}

fn large_source(functions: usize) -> String {
    let mut source = String::from("state \"large.exe\" {}\n\n");
    for index in 0..functions {
        source.push_str(&format!(
            "fn helper{index}(value: u32) -> u32 {{\n    return value + {index}\n}}\n\n"
        ));
    }
    source.push_str(&format!(
        "onDetached {{\n    let selected = helper{}(1)\n    print(selected as String)\n}}\n",
        functions - 1
    ));
    source
}

fn line_character(source: &str, offset: usize) -> (usize, usize) {
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let character = prefix
        .rsplit_once('\n')
        .map_or(prefix.len(), |(_, line)| line.len());
    (line, character)
}

fn measure(name: &str, iterations: usize, warmups: usize, mut operation: impl FnMut()) {
    for _ in 0..warmups {
        operation();
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        operation();
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
    println!(
        "{name}\t{}\t{}",
        nanos_to_micros(median),
        nanos_to_micros(p95)
    );
}

fn nanos_to_micros(nanos: u128) -> String {
    format!("{:.1}", nanos as f64 / 1_000.0)
}
