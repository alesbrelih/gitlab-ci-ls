use std::collections::HashMap;

use lsp_server::RequestId;

pub mod events;
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
pub enum LSPResult {
    Hover(HoverResult),
    Definition(DefinitionResult),
}

#[derive(Debug)]
pub struct GitlabFile {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct LSPConfig {
    pub root_dir: String,
    pub cache_path: String,
    pub package_map: HashMap<String, String>,
}
