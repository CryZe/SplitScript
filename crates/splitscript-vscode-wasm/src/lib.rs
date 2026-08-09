//! Minimal direct WebAssembly ABI for the portable editor adapter.
//!
//! Requests are UTF-8 JSON. Responses use a compact envelope containing a JSON
//! metadata prefix and the generated module as raw bytes:
//!
//! `SSCR | metadata length (u32 LE) | metadata JSON | artifact bytes`
//!
//! Keeping artifact bytes outside JSON avoids base64 and number-array copies.

#![cfg(target_arch = "wasm32")]

use std::{cell::RefCell, ptr, slice};

use serde::Serialize;
use serde_json::{Value, json};
use splitscript::compiler::service::{
    COMPILER_SERVICE_PROTOCOL_VERSION, CompileRequest, CompileServiceError,
    CompileServiceErrorCode, CompilerService, ServiceDiagnostic,
};
use splitscript::tooling::lsp::LanguageServer;
use splitscript::{CompilerIdentity, compiler_identity};

const RESPONSE_MAGIC: &[u8; 4] = b"SSCR";
const LSP_PROTOCOL_VERSION: u32 = 1;

thread_local! {
    static SERVICE: CompilerService = CompilerService::new();
    static RESPONSE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static LANGUAGE_SERVER: RefCell<LanguageServer> = RefCell::new(LanguageServer::default());
    static LSP_RESPONSE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResponseMetadata {
    protocol_version: u32,
    compiler: CompilerIdentity,
    uri: String,
    revision: u64,
    diagnostics: Vec<ServiceDiagnostic>,
    artifact_length: usize,
    error: Option<CompileServiceError>,
}

#[unsafe(no_mangle)]
pub extern "C" fn splitscript_service_protocol_version() -> u32 {
    COMPILER_SERVICE_PROTOCOL_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn splitscript_service_alloc(length: u32) -> u32 {
    let allocation = vec![0_u8; length as usize].into_boxed_slice();
    Box::into_raw(allocation).cast::<u8>() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn splitscript_service_dealloc(pointer: u32, length: u32) {
    let slice = ptr::slice_from_raw_parts_mut(pointer as *mut u8, length as usize);
    // SAFETY: The editor adapter passes only allocations returned by
    // `splitscript_service_alloc`, once, with their original length.
    unsafe { drop(Box::from_raw(slice)) };
}

#[unsafe(no_mangle)]
pub extern "C" fn splitscript_service_compile(pointer: u32, length: u32) {
    // SAFETY: The editor adapter writes exactly `length` initialized bytes to
    // an allocation returned by `splitscript_service_alloc` before this call.
    let request_bytes = unsafe { slice::from_raw_parts(pointer as *const u8, length as usize) };
    let response = match serde_json::from_slice::<CompileRequest>(request_bytes) {
        Ok(request) => SERVICE.with(|service| match service.compile(request) {
            Ok(response) => {
                let artifact = response.artifact.unwrap_or_default();
                encode_response(
                    ResponseMetadata {
                        protocol_version: response.protocol_version,
                        compiler: response.compiler,
                        uri: response.uri,
                        revision: response.revision,
                        diagnostics: response.diagnostics,
                        artifact_length: artifact.len(),
                        error: None,
                    },
                    &artifact,
                )
            }
            Err(error) => encode_error(error),
        }),
        Err(error) => encode_error(CompileServiceError::new(
            CompileServiceErrorCode::InvalidRequest,
            format!("invalid compiler-service request: {error}"),
        )),
    };
    RESPONSE.with(|slot| *slot.borrow_mut() = response);
}

#[unsafe(no_mangle)]
pub extern "C" fn splitscript_service_response_pointer() -> u32 {
    RESPONSE.with(|response| response.borrow().as_ptr() as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn splitscript_service_response_length() -> u32 {
    RESPONSE.with(|response| response.borrow().len() as u32)
}

/// Version of the direct JSON-message ABI used by editor language clients.
#[unsafe(no_mangle)]
pub extern "C" fn splitscript_lsp_protocol_version() -> u32 {
    LSP_PROTOCOL_VERSION
}

/// Handles one UTF-8 JSON-RPC message and stores a JSON array containing every
/// response or notification produced by the language server.
#[unsafe(no_mangle)]
pub extern "C" fn splitscript_lsp_handle(pointer: u32, length: u32) {
    // SAFETY: The editor adapter writes exactly `length` initialized bytes to
    // an allocation returned by `splitscript_service_alloc` before this call.
    let request_bytes = unsafe { slice::from_raw_parts(pointer as *const u8, length as usize) };
    let outgoing = match serde_json::from_slice::<Value>(request_bytes) {
        Ok(message) => LANGUAGE_SERVER.with(|server| server.borrow_mut().handle(message)),
        Err(error) => vec![json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": -32700,
                "message": format!("invalid JSON-RPC message: {error}")
            }
        })],
    };
    let response = serde_json::to_vec(&outgoing).unwrap_or_else(|error| {
        serde_json::to_vec(&vec![json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": -32603,
                "message": format!("could not encode JSON-RPC response: {error}")
            }
        })])
        .unwrap_or_else(|_| b"[]".to_vec())
    });
    LSP_RESPONSE.with(|slot| *slot.borrow_mut() = response);
}

#[unsafe(no_mangle)]
pub extern "C" fn splitscript_lsp_response_pointer() -> u32 {
    LSP_RESPONSE.with(|response| response.borrow().as_ptr() as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn splitscript_lsp_response_length() -> u32 {
    LSP_RESPONSE.with(|response| response.borrow().len() as u32)
}

fn encode_error(error: CompileServiceError) -> Vec<u8> {
    encode_response(
        ResponseMetadata {
            protocol_version: COMPILER_SERVICE_PROTOCOL_VERSION,
            compiler: compiler_identity(),
            uri: String::new(),
            revision: 0,
            diagnostics: Vec::new(),
            artifact_length: 0,
            error: Some(error),
        },
        &[],
    )
}

fn encode_response(metadata: ResponseMetadata, artifact: &[u8]) -> Vec<u8> {
    let (metadata, artifact) = match serde_json::to_vec(&metadata) {
        Ok(metadata) => (metadata, artifact),
        Err(error) => {
            let fallback = ResponseMetadata {
                protocol_version: COMPILER_SERVICE_PROTOCOL_VERSION,
                compiler: compiler_identity(),
                uri: String::new(),
                revision: 0,
                diagnostics: Vec::new(),
                artifact_length: 0,
                error: Some(CompileServiceError::new(
                    CompileServiceErrorCode::Internal,
                    format!("could not encode compiler-service response: {error}"),
                )),
            };
            (
                serde_json::to_vec(&fallback).unwrap_or_else(|_| b"{}".to_vec()),
                &[][..],
            )
        }
    };
    let metadata_length = u32::try_from(metadata.len()).unwrap_or(u32::MAX);
    let mut response = Vec::with_capacity(8 + metadata.len() + artifact.len());
    response.extend_from_slice(RESPONSE_MAGIC);
    response.extend_from_slice(&metadata_length.to_le_bytes());
    response.extend_from_slice(&metadata);
    response.extend_from_slice(artifact);
    response
}
