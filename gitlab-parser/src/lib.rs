use std::collections::HashMap;

use lsp_server::RequestId;
use lsp_types::Diagnostic;

pub mod handlers;
mod parser;

#[derive(Debug)]
pub struct LSPPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug)]
pub struct Range {
    pub start: LSPPosition,
    pub end: LSPPosition,
}

#[derive(Debug)]
pub struct DefinitionResult {
    pub id: RequestId,
    pub locations: Vec<LSPLocation>,
}

#[derive(Debug)]
pub struct CompletionResult {
    pub id: RequestId,
    pub list: Vec<LSPCompletion>,
}

#[derive(Debug)]
pub struct LSPCompletion {
    pub label: String,
    pub details: String,
}

#[derive(Debug)]
pub struct LSPLocation {
    pub uri: String,
    pub range: Range,
}

#[derive(Debug)]
pub struct LSPDiagnosticItem {
    pub range: Range,
    pub severity: String,
    pub message: String,
}

#[derive(Debug)]
pub struct LSPDiagnosticDocument {
    pub uri: String,
    pub items: Vec<LSPDiagnosticItem>,
}

#[derive(Debug)]
pub struct LSPDiagnostic {
    pub documents: Vec<LSPDiagnosticDocument>,
}

#[derive(Debug)]
pub struct HoverResult {
    pub id: RequestId,
    pub content: String,
}

#[derive(Debug)]
pub struct DiagnosticsResult {
    pub id: RequestId,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug)]
pub enum LSPResult {
    Hover(HoverResult),
    Completion(CompletionResult),
    Definition(DefinitionResult),
    Diagnostics(DiagnosticsResult),
}

#[derive(Debug)]
pub struct GitlabFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug)]
pub struct GitlabRootNode {
    pub uri: String,
    pub key: String,
    pub description: String,
}

#[derive(Debug)]
pub struct GitlabExtend {
    pub key: String,
    pub uri: String,
    pub range: Range,
}

#[derive(Clone, Debug)]
pub struct LSPConfig {
    pub root_dir: String,
    pub cache_path: String,
    pub package_map: HashMap<String, String>,
    pub remote_urls: Vec<String>,
}
