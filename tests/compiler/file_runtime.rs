//! Whole-file API integration tests against a deliberately partial WASI host.

use std::collections::HashMap;

#[derive(Default)]
struct FileTestHost {
    files: HashMap<String, Vec<u8>>,
    open_files: HashMap<i32, (String, usize)>,
    opened_paths: Vec<String>,
    closed_descriptors: usize,
    messages: Vec<String>,
    next_descriptor: i32,
    module_mtime: u64,
    module_replacement: Option<Vec<u8>>,
    module_filestat_calls: usize,
}

fn write_memory(caller: &mut wasmtime::Caller<'_, FileTestHost>, pointer: i32, bytes: &[u8]) {
    let memory = caller
        .get_export("memory")
        .and_then(wasmtime::Extern::into_memory)
        .expect("generated modules export memory");
    memory
        .write(caller, pointer as usize, bytes)
        .expect("mock WASI output belongs to guest memory");
}

fn read_memory(
    caller: &mut wasmtime::Caller<'_, FileTestHost>,
    pointer: i32,
    length: i32,
) -> Vec<u8> {
    let memory = caller
        .get_export("memory")
        .and_then(wasmtime::Extern::into_memory)
        .expect("generated modules export memory");
    let mut bytes = vec![0; length as usize];
    memory
        .read(caller, pointer as usize, &mut bytes)
        .expect("mock WASI input belongs to guest memory");
    bytes
}

fn execute(source: &str) -> FileTestHost {
    execute_with_updates(source, Vec::new(), 0)
}

fn execute_with_updates(source: &str, module_file: Vec<u8>, updates: usize) -> FileTestHost {
    execute_with_module_change(source, module_file, None, updates)
}

fn execute_with_module_change(
    source: &str,
    module_file: Vec<u8>,
    module_replacement: Option<Vec<u8>>,
    updates: usize,
) -> FileTestHost {
    use wasmtime::{Config, Engine, ExternType, Linker, Module, Store, Val, ValType};

    let wasm = splitscript::compile(source).expect("file API fixture should compile");
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&wasm)
        .expect("file helpers should produce valid Wasm GC");

    let mut config = Config::new();
    config.wasm_gc(true);
    config.wasm_function_references(true);
    let engine = Engine::new(&config).expect("GC-enabled Wasmtime should initialize");
    let module = Module::new(&engine, wasm).expect("Wasmtime should compile generated Wasm");
    let mut linker: Linker<FileTestHost> = Linker::new(&engine);

    for import in module.imports() {
        let ExternType::Func(function_type) = import.ty() else {
            panic!("SplitScript host ABI imports only functions")
        };
        let module_name = import.module().to_owned();
        let name = import.name().to_owned();
        let host_name = name.clone();
        let result_types = function_type.results().collect::<Vec<_>>();
        linker
            .func_new(
                &module_name,
                &name,
                function_type,
                move |mut caller, parameters, results| {
                    for (result, ty) in results.iter_mut().zip(&result_types) {
                        *result = match ty {
                            ValType::I32 => Val::I32(0),
                            ValType::I64 => Val::I64(0),
                            ValType::F32 => Val::F32(0),
                            ValType::F64 => Val::F64(0),
                            ty => panic!("mock host does not return `{ty}`"),
                        };
                    }
                    match host_name.as_str() {
                        "fd_prestat_get" => {
                            if parameters[0].unwrap_i32() == 3 {
                                let pointer = parameters[1].unwrap_i32();
                                write_memory(&mut caller, pointer, &[0, 0, 0, 0]);
                                write_memory(&mut caller, pointer + 4, &6_u32.to_le_bytes());
                            } else {
                                results[0] = Val::I32(8); // __WASI_ERRNO_BADF
                            }
                        }
                        "fd_prestat_dir_name" => {
                            assert_eq!(parameters[0].unwrap_i32(), 3);
                            assert_eq!(parameters[2].unwrap_i32(), 6);
                            write_memory(&mut caller, parameters[1].unwrap_i32(), b"/mnt/c");
                        }
                        "path_open" => {
                            assert_eq!(parameters[0].unwrap_i32(), 3);
                            assert_eq!(parameters[1].unwrap_i32(), 1);
                            assert_eq!(parameters[4].unwrap_i32(), 0);
                            assert!(matches!(parameters[5].unwrap_i64(), 2 | 0x20_0006));
                            assert_eq!(parameters[6].unwrap_i64(), 0);
                            assert_eq!(parameters[7].unwrap_i32(), 0);
                            let path = String::from_utf8(read_memory(
                                &mut caller,
                                parameters[2].unwrap_i32(),
                                parameters[3].unwrap_i32(),
                            ))
                            .expect("portable test paths are UTF-8");
                            caller.data_mut().opened_paths.push(path.clone());
                            if caller.data().files.contains_key(&path) {
                                let descriptor = caller.data().next_descriptor;
                                caller.data_mut().next_descriptor += 1;
                                caller.data_mut().open_files.insert(descriptor, (path, 0));
                                write_memory(
                                    &mut caller,
                                    parameters[8].unwrap_i32(),
                                    &(descriptor as u32).to_le_bytes(),
                                );
                            } else {
                                results[0] = Val::I32(44); // __WASI_ERRNO_NOENT
                            }
                        }
                        "fd_read" => {
                            assert_eq!(parameters[2].unwrap_i32(), 1);
                            let descriptor = parameters[0].unwrap_i32();
                            let iovec = read_memory(&mut caller, parameters[1].unwrap_i32(), 8);
                            let pointer = i32::from_le_bytes(iovec[0..4].try_into().unwrap());
                            let capacity =
                                u32::from_le_bytes(iovec[4..8].try_into().unwrap()) as usize;
                            let (path, offset) = caller
                                .data()
                                .open_files
                                .get(&descriptor)
                                .cloned()
                                .expect("reads use an open descriptor");
                            if path == "autosplitter/read-error.bin" && offset != 0 {
                                results[0] = Val::I32(5); // __WASI_ERRNO_IO
                                return Ok(());
                            }
                            let file = &caller.data().files[&path];
                            // Force partial reads so the guest must preserve and
                            // append every chunk rather than assuming one call.
                            let length = (file.len() - offset).min(capacity).min(8_191);
                            let chunk = file[offset..offset + length].to_vec();
                            caller.data_mut().open_files.get_mut(&descriptor).unwrap().1 += length;
                            write_memory(&mut caller, pointer, &chunk);
                            write_memory(
                                &mut caller,
                                parameters[3].unwrap_i32(),
                                &(length as u32).to_le_bytes(),
                            );
                        }
                        "fd_seek" => {
                            let descriptor = parameters[0].unwrap_i32();
                            let offset = parameters[1].unwrap_i64();
                            assert_eq!(parameters[2].unwrap_i32(), 0);
                            assert!(offset >= 0);
                            caller
                                .data_mut()
                                .open_files
                                .get_mut(&descriptor)
                                .expect("seeks use an open descriptor")
                                .1 = offset as usize;
                            write_memory(
                                &mut caller,
                                parameters[3].unwrap_i32(),
                                &(offset as u64).to_le_bytes(),
                            );
                        }
                        "fd_filestat_get" => {
                            let descriptor = parameters[0].unwrap_i32();
                            let (path, _) = caller
                                .data()
                                .open_files
                                .get(&descriptor)
                                .cloned()
                                .expect("filestat uses an open descriptor");
                            if path == "autosplitter/module.bin" {
                                caller.data_mut().module_filestat_calls += 1;
                                if caller.data().module_filestat_calls == 2
                                    && let Some(replacement) =
                                        caller.data_mut().module_replacement.take()
                                {
                                    caller.data_mut().files.insert(path.clone(), replacement);
                                    caller.data_mut().module_mtime += 1;
                                }
                            }
                            let size = caller.data().files[&path].len() as u64;
                            let mtime = if path == "autosplitter/module.bin" {
                                caller.data().module_mtime
                            } else {
                                1
                            };
                            let pointer = parameters[1].unwrap_i32();
                            write_memory(&mut caller, pointer + 32, &size.to_le_bytes());
                            write_memory(&mut caller, pointer + 48, &mtime.to_le_bytes());
                        }
                        "fd_close" => {
                            let descriptor = parameters[0].unwrap_i32();
                            assert!(caller.data_mut().open_files.remove(&descriptor).is_some());
                            caller.data_mut().closed_descriptors += 1;
                        }
                        "runtime_print_message" => {
                            let bytes = read_memory(
                                &mut caller,
                                parameters[0].unwrap_i32(),
                                parameters[1].unwrap_i32(),
                            );
                            caller
                                .data_mut()
                                .messages
                                .push(String::from_utf8(bytes).expect("printed text is UTF-8"));
                        }
                        "process_attach" => results[0] = Val::I64(1),
                        "process_is_open" => results[0] = Val::I32(1),
                        "process_get_module_address" => results[0] = Val::I64(0x1000),
                        "process_get_module_size" => results[0] = Val::I64(0x1000),
                        "process_get_module_path" => {
                            let path = b"/mnt/c/autosplitter/module.bin";
                            let output = parameters[3].unwrap_i32();
                            if output != 0 {
                                write_memory(&mut caller, output, path);
                            }
                            write_memory(
                                &mut caller,
                                parameters[4].unwrap_i32(),
                                &(path.len() as u32).to_le_bytes(),
                            );
                            results[0] = Val::I32(1);
                        }
                        _ => {}
                    }
                    Ok(())
                },
            )
            .expect("mock host import should be unique");
    }

    let mut split_utf8 = vec![b'a'; 8_190];
    split_utf8.extend_from_slice("é終".as_bytes());
    let mut binary = (0..70_003).map(|index| index as u8).collect::<Vec<_>>();
    binary[42] = 0;
    let mut store = Store::new(
        &engine,
        FileTestHost {
            files: HashMap::from([
                ("autosplitter/configuration.txt".to_owned(), split_utf8),
                ("absolute.bin".to_owned(), binary),
                (
                    "autosplitter/invalid.txt".to_owned(),
                    vec![0xf0, 0x28, 0x8c, 0x28],
                ),
                ("autosplitter/read-error.bin".to_owned(), vec![42; 9_000]),
                ("autosplitter/empty.txt".to_owned(), Vec::new()),
                ("autosplitter/module.bin".to_owned(), module_file.clone()),
            ]),
            next_descriptor: 4,
            module_mtime: 1,
            module_replacement,
            ..FileTestHost::default()
        },
    );
    let instance = linker
        .instantiate(&mut store, &module)
        .expect("generated module should instantiate against mock WASI");
    instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .unwrap()
        .call(&mut store, ())
        .unwrap();
    if updates != 0 {
        let update = instance
            .get_typed_func::<(), ()>(&mut store, "update")
            .expect("stateful fixtures export update");
        for _ in 0..updates {
            update.call(&mut store, ()).unwrap();
        }
    }
    assert!(
        store.data().open_files.is_empty(),
        "every opened file is closed"
    );
    store.into_data()
}

#[test]
fn module_md5_restarts_when_the_file_changes_between_polls() {
    let host = execute_with_module_change(
        r#"
            state "game.exe" {}

            onAttach {
                let executable = await process.mainModule()
                let fingerprint = (await executable.md5()) else "hash-error"
                print(fingerprint)
            }
        "#,
        b"abc".to_vec(),
        Some(b"def".to_vec()),
        8,
    );

    assert_eq!(host.messages, ["4ED9407630EB1000C0F6B63842DEFA7D"]);
    assert_eq!(host.closed_descriptors, 2);
    assert!(host.open_files.is_empty());
}

#[test]
fn module_md5_hashes_exact_file_bytes_as_a_cooperative_future() {
    let source = r#"
            state "game.exe" {}

            onAttach {
                let executable = await process.mainModule()
                let fingerprint = (await executable.md5()) else "hash-error"
                print(fingerprint)
            }
        "#;
    let cases = [
        (Vec::new(), "D41D8CD98F00B204E9800998ECF8427E", 1),
        (b"abc".to_vec(), "900150983CD24FB0D6963F7D28E17F72", 1),
        (vec![b'a'; 56], "3B0C8AC703F828B04C6C197006D17218", 1),
        (vec![b'a'; 600_000], "09F901B937EDAF5A12718C4753F563F4", 2),
    ];

    for (bytes, expected, expected_polls) in cases {
        let host = execute_with_updates(source, bytes, 8);
        assert_eq!(host.messages, [expected]);
        assert_eq!(host.closed_descriptors, expected_polls);
        assert!(host.open_files.is_empty());
    }
}

#[test]
fn whole_file_apis_require_absolute_paths_read_partially_and_validate_utf8() {
    let host = execute(
        r#"
            state "game.exe" {}

            setup {
                let text = File.readAllText("/mnt/c/autosplitter/configuration.txt") else "text-error"
                print(text.byteLength())
                print(text.byteAt(8190) else 0)

                let bytes = File.readAllBytes("/mnt/c/absolute.bin") else []
                print(bytes.length())
                print(bytes[42])
                print(bytes[70002])

                let invalid = File.readAllText("/mnt/c/autosplitter/invalid.txt") else "invalid"
                print(invalid)
                let missing = File.readAllBytes("/mnt/c/autosplitter/missing.bin") else []
                print(missing.length())
                let incomplete = File.readAllBytes("/mnt/c/autosplitter/read-error.bin") else []
                print(incomplete.length())
                let empty = File.readAllText("/mnt/c/autosplitter/empty.txt") else "empty-error"
                print(empty.byteLength())

                let relative = File.readAllBytes("absolute.bin") else []
                print(relative.length())
            }
        "#,
    );

    assert_eq!(
        host.messages,
        [
            "8195", "195", "70003", "0", "114", "invalid", "0", "0", "0", "0"
        ]
    );
    assert_eq!(
        host.opened_paths,
        [
            "autosplitter/configuration.txt",
            "absolute.bin",
            "autosplitter/invalid.txt",
            "autosplitter/missing.bin",
            "autosplitter/read-error.bin",
            "autosplitter/empty.txt",
        ]
    );
    assert_eq!(host.closed_descriptors, 5);
}

#[test]
fn filesystem_imports_are_demand_driven() {
    let imports = |source| {
        let wasm = splitscript::compile(source).expect("fixture should compile");
        wasmparser::Parser::new(0)
            .parse_all(&wasm)
            .filter_map(Result::ok)
            .filter_map(|payload| match payload {
                wasmparser::Payload::ImportSection(section) => Some(section),
                _ => None,
            })
            .flat_map(|section| section.into_imports().filter_map(Result::ok))
            .map(|import| (import.module.to_owned(), import.name.to_owned()))
            .collect::<Vec<_>>()
    };
    let unused = imports(r#"state "game.exe" {}"#);
    assert!(
        !unused
            .iter()
            .any(|(module, _)| module == "wasi_snapshot_preview1")
    );

    let used = imports(
        r#"
            state "game.exe" {}
            setup { File.readAllText("/mnt/settings.txt") else "" }
        "#,
    );
    for name in [
        "fd_prestat_get",
        "fd_prestat_dir_name",
        "path_open",
        "fd_read",
        "fd_close",
    ] {
        assert!(
            used.iter()
                .any(|(module, import)| { module == "wasi_snapshot_preview1" && import == name }),
            "missing demand-driven WASI import `{name}`: {used:?}"
        );
    }

    let fingerprint = imports(
        r#"
            state "game.exe" {}
            onAttach {
                let executable = await process.mainModule()
                print((await executable.md5()) else "")
            }
        "#,
    );
    for name in [
        "fd_prestat_get",
        "fd_prestat_dir_name",
        "path_open",
        "fd_read",
        "fd_seek",
        "fd_filestat_get",
        "fd_close",
    ] {
        assert!(
            fingerprint
                .iter()
                .any(|(module, import)| { module == "wasi_snapshot_preview1" && import == name }),
            "missing fingerprint WASI import `{name}`: {fingerprint:?}"
        );
    }
}
