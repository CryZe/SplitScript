//! Repeatable compiler-query and in-process LSP latency/retained-heap baseline.
//!
//! Run with `cargo run --release --example tooling_baseline -- 500 100`.
//! Append `--root-effects` to measure repeated root completion in detached contexts.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

use serde_json::json;
use splitscript::tooling::{database::CompilerDatabase, lsp::LanguageServer};

const DEFAULT_FUNCTIONS: usize = 500;
const DEFAULT_ITERATIONS: usize = 100;
const WARMUP_ITERATIONS: usize = 20;

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);

struct TrackingAllocator;

// SAFETY: Every operation delegates to `System` with the original pointer and
// layout. The atomics observe byte counts only and do not affect allocation.
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: This forwards the allocation request unchanged.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            allocation_added(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: This forwards the allocation request unchanged.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            allocation_added(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        ALLOCATED_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: This forwards the pointer and its original layout unchanged.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: This forwards the pointer, its original layout, and new size.
        let new_pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !new_pointer.is_null() {
            match new_size.cmp(&layout.size()) {
                std::cmp::Ordering::Greater => allocation_added(new_size - layout.size()),
                std::cmp::Ordering::Less => {
                    ALLOCATED_BYTES.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
                }
                std::cmp::Ordering::Equal => {}
            }
        }
        new_pointer
    }
}

fn allocation_added(bytes: usize) {
    let allocated = ALLOCATED_BYTES.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK_ALLOCATED_BYTES.fetch_max(allocated, Ordering::Relaxed);
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    let functions = parse_positive(arguments.next(), DEFAULT_FUNCTIONS, "function count");
    let iterations = parse_positive(arguments.next(), DEFAULT_ITERATIONS, "iteration count");
    let root_effects = arguments.next().is_some_and(|argument| {
        assert_eq!(argument, "--root-effects", "unknown benchmark mode");
        true
    });
    assert!(arguments.next().is_none(), "unexpected extra arguments");

    let fixtures = [
        Fixture::new("small", small_source(), "current.position", "point.x"),
        Fixture::new(
            "lunistice",
            include_str!("lunistice.split").to_owned(),
            "current.",
            "current.",
        ),
        Fixture::new(
            "generated_large",
            large_source(functions),
            &format!("helper{}", functions - 1),
            "point.x",
        ),
    ];

    // Initialize the process-wide standard-library graph before measuring
    // source-owned work. Process startup is a separate product concern and
    // would otherwise appear only as an outlier in the first fixture.
    let mut bootstrap = CompilerDatabase::new("state \"bootstrap.exe\" {}");
    black_box(bootstrap.diagnostics());
    drop(bootstrap);

    println!("profile=release");
    println!(
        "platform={}-{} logical_cpus={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(1, usize::from)
    );
    println!(
        "large_functions={functions} warmup_iterations={WARMUP_ITERATIONS} measured_iterations={iterations}"
    );
    println!(
        "fixture\tsource_bytes\tquery\tmedian_us\tp95_us\tretained_delta_bytes\tpeak_delta_bytes"
    );

    if root_effects {
        run_root_effects(functions, iterations);
        return;
    }

    for fixture in &fixtures {
        run_fixture(fixture, iterations);
    }

    println!("retained fixture\tstate\tretained_bytes\tpeak_delta_bytes");
    for fixture in &fixtures {
        report_retained_states(fixture);
    }
}

fn run_root_effects(functions: usize, iterations: usize) {
    use std::fmt::Write;

    let mut declarations = String::from(
        "state \"game.exe\" { level: u32 at 0x100 }\n\
         fn readsState() { return current.level }\n\
         fn relay() { return readsState() }\n",
    );
    for index in 0..functions {
        writeln!(
            declarations,
            "fn helper{index}(value: u32) {{ return value + {index} }}"
        )
        .unwrap();
    }
    for (name, suffix) in [
        ("root_valid", "onDetach { helper0(0) }"),
        ("root_partial", "onDetach { hel }"),
        (
            "root_failed_repair",
            "fn broken() { missing }\nonDetach { hel }",
        ),
    ] {
        let mut fixture = Fixture::new(name, format!("{declarations}{suffix}\n"), "hel", "hel");
        fixture.root_offset = fixture.member_offset;
        let query = |database: &mut CompilerDatabase, fixture: &Fixture| {
            black_box(
                database
                    .completions(fixture.root_offset)
                    .expect("root completion should recover"),
            );
        };
        measure_database_edit(&fixture, "database_edit_root_effects", iterations, query);
        let mut database = CompilerDatabase::new(fixture.source.clone());
        measure(
            &fixture,
            "database_warm_root_effects",
            iterations,
            WARMUP_ITERATIONS,
            || {
                query(&mut database, &fixture);
            },
        );
        report_retained_state(&fixture, "root_effects", query);
    }
}

fn run_fixture(fixture: &Fixture, iterations: usize) {
    measure(fixture, "database_cold_diagnostics", iterations, 0, || {
        let mut database = CompilerDatabase::new(fixture.source.clone());
        black_box(database.diagnostics());
    });

    measure_database_edit(
        fixture,
        "database_edit_diagnostics",
        iterations,
        |database, _| {
            black_box(database.diagnostics());
        },
    );
    measure_database_edit(
        fixture,
        "database_edit_root_completion",
        iterations,
        |database, fixture| {
            black_box(
                database
                    .completions(fixture.root_offset)
                    .expect("root completion must succeed"),
            );
        },
    );
    measure_database_edit(
        fixture,
        "database_edit_member_completion",
        iterations,
        |database, fixture| {
            black_box(
                database
                    .completions(fixture.member_offset)
                    .expect("member completion must succeed"),
            );
        },
    );
    measure_database_edit(
        fixture,
        "database_edit_hover",
        iterations,
        |database, fixture| {
            black_box(
                database
                    .hover(fixture.hover_offset)
                    .expect("hover query must succeed"),
            );
        },
    );
    measure_database_edit(
        fixture,
        "database_edit_semantic_tokens",
        iterations,
        |database, _| {
            black_box(
                database
                    .semantic_highlights()
                    .expect("semantic highlighting must succeed"),
            );
        },
    );

    let mut database = CompilerDatabase::new(fixture.source.clone());
    measure(
        fixture,
        "database_warm_query_sequence",
        iterations,
        WARMUP_ITERATIONS,
        || {
            black_box(database.diagnostics());
            black_box(
                database
                    .completions(fixture.root_offset)
                    .expect("root completion must succeed"),
            );
            black_box(
                database
                    .completions(fixture.member_offset)
                    .expect("member completion must succeed"),
            );
            black_box(
                database
                    .hover(fixture.hover_offset)
                    .expect("hover query must succeed"),
            );
            black_box(
                database
                    .semantic_highlights()
                    .expect("semantic highlighting must succeed"),
            );
        },
    );

    measure_lsp_did_change(fixture, iterations);
    measure(fixture, "lsp_restart_to_hover", iterations, 0, || {
        let mut server = initialized_server();
        let diagnostics = server.handle(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": fixture.uri,
                    "languageId": "splitscript",
                    "version": 1,
                    "text": fixture.source,
                }
            }
        }));
        assert_publish_diagnostics(&diagnostics);
        let response = server.handle(hover_request(fixture, 2));
        assert_eq!(response.len(), 1, "hover must produce one response");
        black_box(response);
    });
}

fn measure_database_edit(
    fixture: &Fixture,
    name: &str,
    iterations: usize,
    mut query: impl FnMut(&mut CompilerDatabase, &Fixture),
) {
    let mut database = CompilerDatabase::new(fixture.source.clone());
    query(&mut database, fixture);
    let mut edited = false;
    measure(fixture, name, iterations, WARMUP_ITERATIONS, || {
        edited = !edited;
        assert!(database.set_source(if edited {
            fixture.edited_source.clone()
        } else {
            fixture.source.clone()
        }));
        query(&mut database, fixture);
    });
}

fn measure_lsp_did_change(fixture: &Fixture, iterations: usize) {
    let mut server = initialized_server();
    let diagnostics = server.handle(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": fixture.uri,
                "languageId": "splitscript",
                "version": 1,
                "text": fixture.source,
            }
        }
    }));
    assert_publish_diagnostics(&diagnostics);

    let mut version = 1_u64;
    let mut edited = false;
    measure(
        fixture,
        "lsp_did_change_to_diagnostics",
        iterations,
        WARMUP_ITERATIONS,
        || {
            version += 1;
            edited = !edited;
            let diagnostics = server.handle(json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didChange",
                "params": {
                    "textDocument": { "uri": fixture.uri, "version": version },
                    "contentChanges": [{
                        "text": if edited { &fixture.edited_source } else { &fixture.source },
                    }]
                }
            }));
            assert_publish_diagnostics(&diagnostics);
            black_box(diagnostics);
        },
    );
}

fn initialized_server() -> LanguageServer {
    let mut server = LanguageServer::default();
    let response = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    }));
    assert_eq!(response.len(), 1, "initialize must produce one response");
    server
}

fn hover_request(fixture: &Fixture, request_id: u64) -> serde_json::Value {
    let (line, character) = line_character(&fixture.source, fixture.hover_offset);
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": fixture.uri },
            "position": { "line": line, "character": character }
        }
    })
}

fn assert_publish_diagnostics(messages: &[serde_json::Value]) {
    assert_eq!(messages.len(), 1, "source change must publish diagnostics");
    assert_eq!(
        messages[0]["method"], "textDocument/publishDiagnostics",
        "source change must publish diagnostics"
    );
}

fn report_retained_states(fixture: &Fixture) {
    report_retained_state(fixture, "diagnostics", |database, _| {
        black_box(database.diagnostics());
    });
    report_retained_state(fixture, "root_completion", |database, fixture| {
        black_box(
            database
                .completions(fixture.root_offset)
                .expect("root completion must succeed"),
        );
    });
    report_retained_state(fixture, "member_completion", |database, fixture| {
        black_box(
            database
                .completions(fixture.member_offset)
                .expect("member completion must succeed"),
        );
    });
    report_retained_state(fixture, "hover", |database, fixture| {
        black_box(
            database
                .hover(fixture.hover_offset)
                .expect("hover query must succeed"),
        );
    });
    report_retained_state(fixture, "semantic_tokens", |database, _| {
        black_box(
            database
                .semantic_highlights()
                .expect("semantic highlighting must succeed"),
        );
    });
    report_retained_state(fixture, "warm_query_sequence", |database, fixture| {
        black_box(database.diagnostics());
        black_box(
            database
                .completions(fixture.root_offset)
                .expect("root completion must succeed"),
        );
        black_box(
            database
                .completions(fixture.member_offset)
                .expect("member completion must succeed"),
        );
        black_box(
            database
                .hover(fixture.hover_offset)
                .expect("hover query must succeed"),
        );
        black_box(
            database
                .semantic_highlights()
                .expect("semantic highlighting must succeed"),
        );
    });
}

fn report_retained_state(
    fixture: &Fixture,
    name: &str,
    query: impl FnOnce(&mut CompilerDatabase, &Fixture),
) {
    let allocated_before = ALLOCATED_BYTES.load(Ordering::Relaxed);
    PEAK_ALLOCATED_BYTES.store(allocated_before, Ordering::Relaxed);
    let mut database = CompilerDatabase::new(fixture.source.clone());
    query(&mut database, fixture);
    let allocated_after = ALLOCATED_BYTES.load(Ordering::Relaxed);
    let peak = PEAK_ALLOCATED_BYTES.load(Ordering::Relaxed);
    println!(
        "{}\t{name}\t{}\t{}",
        fixture.name,
        allocated_after.saturating_sub(allocated_before),
        peak.saturating_sub(allocated_before),
    );
    drop(database);
}

struct Fixture {
    name: &'static str,
    uri: String,
    source: String,
    edited_source: String,
    root_offset: usize,
    member_offset: usize,
    hover_offset: usize,
}

impl Fixture {
    fn new(name: &'static str, mut source: String, hover: &str, member: &str) -> Self {
        if !source.ends_with('\n') {
            source.push('\n');
        }
        let root_offset = source.len();
        let hover_offset = source
            .rfind(hover)
            .unwrap_or_else(|| panic!("fixture `{name}` must contain hover marker `{hover}`"));
        let member_start = source
            .rfind(member)
            .unwrap_or_else(|| panic!("fixture `{name}` must contain member marker `{member}`"));
        let member_offset = member_start
            + member
                .rfind('.')
                .map_or(member.len(), |dot| dot.saturating_add(1));
        let edited_source = format!("{source}// full-sync edit\n");
        Self {
            name,
            uri: format!("file:///tooling-baseline-{name}.split"),
            source,
            edited_source,
            root_offset,
            member_offset,
            hover_offset,
        }
    }
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

fn small_source() -> String {
    r#"struct Position {
    x: u32,
    y: u32,
}

state "small.exe" {
    position: Position at 0x100,
}

whileAttached {
    let point = current.position
    print(point.x)
}
"#
    .to_owned()
}

fn large_source(functions: usize) -> String {
    let mut source = String::from(
        "struct Position {\n    x: u32,\n    y: u32,\n}\n\nstate \"large.exe\" {\n    position: Position at 0x100,\n}\n\n",
    );
    for index in 0..functions {
        source.push_str(&format!(
            "fn helper{index}(value: u32) -> u32 {{\n    return value + {index}\n}}\n\n"
        ));
    }
    source.push_str(&format!(
        "whileAttached {{\n    let point = current.position\n    let selected = helper{}(point.x)\n    print(selected)\n}}\n",
        functions - 1
    ));
    source
}

fn line_character(source: &str, offset: usize) -> (usize, usize) {
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let line_text = prefix.rsplit_once('\n').map_or(prefix, |(_, line)| line);
    (line, line_text.encode_utf16().count())
}

fn measure(
    fixture: &Fixture,
    name: &str,
    iterations: usize,
    warmups: usize,
    mut operation: impl FnMut(),
) {
    for _ in 0..warmups {
        operation();
    }
    let mut samples = Vec::with_capacity(iterations);
    let allocated_before = ALLOCATED_BYTES.load(Ordering::Relaxed);
    PEAK_ALLOCATED_BYTES.store(allocated_before, Ordering::Relaxed);
    for _ in 0..iterations {
        let start = Instant::now();
        operation();
        samples.push(start.elapsed().as_nanos());
    }
    let allocated_after = ALLOCATED_BYTES.load(Ordering::Relaxed);
    let peak = PEAK_ALLOCATED_BYTES.load(Ordering::Relaxed);
    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
    println!(
        "{}\t{}\t{name}\t{}\t{}\t{}\t{}",
        fixture.name,
        fixture.source.len(),
        nanos_to_micros(median),
        nanos_to_micros(p95),
        signed_delta(allocated_after, allocated_before),
        peak.saturating_sub(allocated_before),
    );
}

fn signed_delta(after: usize, before: usize) -> i128 {
    after as i128 - before as i128
}

fn nanos_to_micros(nanos: u128) -> String {
    format!("{:.1}", nanos as f64 / 1_000.0)
}
