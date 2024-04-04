use std::collections::HashMap;

use lsp_server::RequestId;
use lsp_types::Diagnostic;

pub mod git;
pub mod handlers;
pub mod parser;
pub mod parser_utils;
pub mod treesitter;

#[derive(Debug, Default)]
pub struct LSPPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Default)]
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
pub struct ReferencesResult {
    pub id: RequestId,
    pub locations: Vec<GitlabElement>,
}

#[derive(Debug)]
pub struct CompletionResult {
    pub id: RequestId,
    pub list: Vec<LSPCompletion>,
}

#[derive(Debug)]
pub struct LSPCompletion {
    pub label: String,
    pub details: Option<String>,
    pub location: LSPLocation,
}

#[derive(Debug, Default)]
pub struct LSPLocation {
    pub uri: String,
    pub range: Range,
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
    References(ReferencesResult),
}

#[derive(Debug, Clone)]
pub struct GitlabFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Default)]
pub struct GitlabElement {
    pub key: String,
    pub content: Option<String>,
    pub uri: String,
    pub range: Range,
}

#[derive(Debug)]
pub struct ParseResults {
    pub files: Vec<GitlabFile>,
    pub nodes: Vec<GitlabElement>,
    pub stages: Vec<GitlabElement>,
    pub variables: Vec<GitlabElement>,
}

#[derive(Clone, Debug)]
pub struct LSPConfig {
    pub root_dir: String,
    pub cache_path: String,
    pub package_map: HashMap<String, String>,
    pub remote_urls: Vec<String>,
}

#[derive(Debug)]
pub struct Include {
    pub path: String,
}
#[derive(Debug, Default)]
pub struct RemoteInclude {
    pub project: Option<String>,
    pub reference: Option<String>,
    pub file: Option<String>,
}

impl RemoteInclude {
    pub fn is_valid(&self) -> bool {
        self.project.is_some() && self.reference.is_some() && self.file.is_some()
    }
}

#[derive(Debug, Default)]
pub struct IncludeInformation {
    pub remote: Option<RemoteInclude>,
    pub remote_url: Option<Include>,
    pub local: Option<Include>,
}

#[derive(Debug)]
pub struct NodeDefinition {
    pub name: String,
}
