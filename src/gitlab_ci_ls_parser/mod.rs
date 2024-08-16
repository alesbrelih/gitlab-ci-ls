use std::collections::HashMap;

use lsp_server::RequestId;
use lsp_types::{Diagnostic, Position, TextEdit, Url};

pub mod fs_utils;
pub mod git;
pub mod handlers;
pub mod messages;
pub mod parser;
pub mod parser_utils;
pub mod treesitter;
pub mod treesitter_queries;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct LSPPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Default, Clone, PartialEq)]
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
pub struct PrepareRenameResult {
    pub id: RequestId,
    pub range: Option<Range>,
    pub err: Option<String>,
}

#[derive(Debug)]
pub struct RenameResult {
    pub id: RequestId,
    pub edits: Option<HashMap<Url, Vec<TextEdit>>>,
    pub err: Option<String>,
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
pub struct DiagnosticsNotification {
    pub uri: Url,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug)]
pub enum LSPResult {
    Hover(HoverResult),
    Completion(CompletionResult),
    Definition(DefinitionResult),
    Diagnostics(DiagnosticsNotification),
    References(ReferencesResult),
    PrepareRename(PrepareRenameResult),
    Rename(RenameResult),
    Error(anyhow::Error),
}

#[derive(Debug, Clone)]
pub struct GitlabFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Default, Clone)]
pub struct GitlabElement {
    pub key: String,
    pub content: Option<String>,
    pub uri: String,
    pub range: Range,
}

#[derive(Debug, Default, Clone)]
pub struct GitlabInputElement {
    pub key: String,
    pub content: Option<String>,
    pub uri: String,
    pub range: Range,
    pub value_plain: Option<GitlabElement>,
    // not yet supported in logic because not sure what is actually supported
    // and I don't want to overengineer from start
    pub value_block: Option<GitlabElement>,
}

#[derive(Debug, Default, Clone)]
pub struct GitlabComponentElement {
    pub key: String,
    pub content: Option<String>,
    pub uri: String,
    pub range: Range,
    pub inputs: Vec<GitlabInputElement>,
}

#[derive(Debug)]
pub struct ParseResults {
    pub files: Vec<GitlabFile>,
    pub nodes: Vec<GitlabElement>,
    pub stages: Vec<GitlabElement>,
    pub components: Vec<Component>,
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
        self.project.is_some() && self.file.is_some()
    }
}

#[derive(Debug, Default)]
pub struct IncludeInformation {
    pub remote: Option<RemoteInclude>,
    pub remote_url: Option<Include>,
    pub local: Option<Include>,
    pub basic: Option<Include>,
    pub component: Option<Component>,
}

#[derive(Debug, Default)]
pub struct RuleReference {
    pub node: String,
}

#[derive(Debug)]
pub struct NodeDefinition {
    pub name: String,
}

#[derive(Debug, Default, Clone)]
pub struct ComponentInputValuePlain {
    value: String,
    hovered: bool,
}

#[derive(Debug, Default, Clone)]
pub struct ComponentInputValueBlock {
    value: String,
    hovered: bool,
}

#[derive(Debug, Default, Clone)]
pub struct ComponentInput {
    pub key: String,
    pub default: Option<serde_yaml::Value>,
    pub description: Option<String>,
    pub options: Option<Vec<String>>,
    pub regex: Option<String>,
    pub prop_type: Option<String>,
    pub hovered: bool,
    pub value_plain: ComponentInputValuePlain,
    pub value_block: ComponentInputValueBlock,
}

impl ComponentInput {
    pub fn autocomplete_details(&self) -> String {
        let mut details = String::new();

        if let Some(d) = &self.description {
            details = format!(
                "## Description: 
{d}
"
            );
        }

        if let Some(d) = &self.prop_type {
            details = format!(
                "{}
## Type: 
{}
",
                details,
                d.as_str()
            );
        }

        if let Some(d) = &self.default {
            details = format!(
                "{}
## Default: 
{}
",
                details,
                d.as_str().unwrap_or_default()
            );
        }

        if let Some(d) = &self.regex {
            details = format!(
                "{}
## Regex: 
{}
",
                details,
                d.as_str()
            );
        }

        details
    }
}

#[derive(Debug, Default)]
pub struct Component {
    pub uri: String,
    pub local_path: String,
    pub inputs: Vec<ComponentInput>,
}

const DEFAULT_BRANCH_SUBFOLDER: &str = "default";
