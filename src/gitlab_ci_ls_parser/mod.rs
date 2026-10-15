use std::collections::HashMap;

use lsp_server::RequestId;
use lsp_types::{Diagnostic, TextEdit, Url};
use serde::{Deserialize, Deserializer, Serialize};

#[cfg(test)]
mod component_e2e_tests;
pub mod components;
pub mod fs_utils;
pub mod git;
pub mod handlers;
pub mod messages;
pub mod parser;
pub mod parser_utils;
pub mod treesitter;
pub mod treesitter_queries;

#[derive(Debug, Default, Clone, PartialEq, Hash)]
pub struct LSPPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Hash)]
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

#[derive(Debug, Default, Clone, Hash, PartialEq)]
pub struct GitlabElement {
    pub key: String,
    pub content: Option<String>,
    pub uri: String,
    pub range: Range,
}

#[derive(Debug, Default, Clone, Hash)]
pub struct GitlabFileElements {
    pub uri: String,
    pub elements: Vec<GitlabElement>,
}

#[derive(Debug, Default, Clone, Hash, PartialEq)]
pub struct GitlabElementWithParentAndLvl {
    pub el: GitlabElement,
    pub parents: String,
    pub lvl: usize,
}

#[derive(Debug, Default, Clone)]
pub struct GitlabCacheElement {
    pub key: String,
    pub content: Option<String>,
    pub uri: String,
    pub range: Range,
    pub cache_items: Vec<GitlabElement>,
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
    // range of the component reference itself (`component: <this>`)
    pub key_range: Range,
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
pub struct LSPExperimental {
    pub dependencies_autocomplete_stage_filtering: bool,
}

#[derive(Clone, Debug)]
pub struct LSPConfig {
    pub root_dir: String,
    pub cache_path: String,
    pub package_map: HashMap<String, String>,
    pub remote_urls: Vec<String>,
    pub variables: HashMap<String, String>,
    pub experimental: LSPExperimental,
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
    pub value: String,
    pub hovered: bool,
}

#[derive(Debug, Default, Clone)]
pub struct ComponentInputValueBlock {
    pub value: String,
    pub hovered: bool,
}

#[derive(Debug, Default, Clone)]
pub struct ComponentInput {
    pub key: String,
    pub default: Option<serde_yaml::Value>,
    pub description: Option<String>,
    pub options: Option<Vec<String>>,
    pub regex: Option<String>,
    pub prop_type: Option<String>,
    // `spec:inputs:<input>:rules` can provide defaults/options conditionally,
    // so we can't statically decide whether the input is required.
    pub has_rules: bool,
    // location of the input key inside the component template `spec:inputs` header
    pub definition_range: Option<Range>,
    pub hovered: bool,
    pub value_plain: ComponentInputValuePlain,
    pub value_block: ComponentInputValueBlock,
}

impl ComponentInput {
    /// Inputs without a default value have to be provided by the caller.
    pub fn is_required(&self) -> bool {
        self.default.is_none() && !self.has_rules
    }

    /// Type of the input; GitLab defaults to `string` when it isn't set.
    pub fn input_type(&self) -> &str {
        self.prop_type.as_deref().unwrap_or("string")
    }

    pub fn default_as_string(&self) -> Option<String> {
        self.default.as_ref().map(yaml_value_to_inline_string)
    }

    pub fn autocomplete_details(&self) -> String {
        let mut details = String::new();

        if let Some(d) = &self.description {
            details = format!(
                "## Description:
{d}
"
            );
        }

        details = format!(
            "{}
## Type:
{}
",
            details,
            self.input_type()
        );

        match self.default_as_string() {
            Some(d) => {
                details = format!(
                    "{details}
## Default:
{d}
"
                );
            }
            None => {
                details = format!(
                    "{details}
## Required
"
                );
            }
        }

        if let Some(options) = &self.options {
            details = format!(
                "{}
## Options:
{}
",
                details,
                options.join(", ")
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

/// Renders a YAML value on a single line, e.g. for hover and completion details.
pub fn yaml_value_to_inline_string(value: &serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Null => "null".to_string(),
        other => serde_json::to_string(other).unwrap_or_else(|_| {
            serde_yaml::to_string(other)
                .unwrap_or_default()
                .trim()
                .to_string()
        }),
    }
}

#[derive(Debug, Default)]
pub struct Component {
    pub uri: String,
    pub local_path: String,
    pub inputs: Vec<ComponentInput>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ComponentSpecInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default: Option<serde_yaml::Value>, // Can be any type (string, number, boolean)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    options: Option<Vec<serde_yaml::Value>>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    type_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    regex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rules: Option<serde_yaml::Value>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ComponentSpecHeader {
    // Mapping keeps the declaration order which is nicer for completion and hover.
    // Values are kept raw because an input without attributes (`my_input:`) is valid
    // and means a required string input.
    #[serde(default)]
    inputs: Option<serde_yaml::Mapping>,
    // Values that can be used with `$[[ component.<value> ]]` interpolation.
    #[serde(default)]
    component: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ComponentSpec {
    #[serde(default)]
    spec: Option<ComponentSpecHeader>,
}

/// Parsed `spec:` header of a file that can be included with inputs (components).
#[derive(Debug, Default, Clone)]
pub struct ParsedComponentSpec {
    pub inputs: Vec<ComponentInput>,
    pub component_context: Vec<String>,
}

/// Values that can be exposed through `spec:component` and used as `$[[ component.<value> ]]`.
pub const COMPONENT_CONTEXT_VALUES: [(&str, &str); 4] = [
    ("name", "The component name."),
    ("sha", "The commit SHA of the component."),
    (
        "version",
        "The resolved semantic version of the component (empty when not included by version).",
    ),
    (
        "reference",
        "The original reference used to include the component, e.g. `1.0`, `~latest` or a branch.",
    ),
];

/// Functions that can be applied in interpolation blocks, e.g. `$[[ inputs.x | expand_vars ]]`.
pub const INTERPOLATION_FUNCTIONS: [(&str, &str); 3] = [
    (
        "expand_vars",
        "Expands CI/CD variables referenced in the input value.",
    ),
    (
        "truncate",
        "`truncate(offset, length)` shortens the input value.",
    ),
    (
        "posix_escape",
        "Escapes POSIX shell special characters in the input value.",
    ),
];

#[derive(Debug, Serialize, Deserialize)]
struct IncludeNode {
    include: Vec<IncludeItem>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)] // This attribute allows for different structs in the same Vec
pub enum IncludeItem {
    Project(Project),
    Local(Local),
    Remote(Remote),
    Basic(String),
    Component(ComponentInclude),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)] // This attribute allows for different structs in the same Vec
pub enum ProjectFile {
    Single(String),
    Multi(Vec<String>),
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(clippy::struct_field_names)]
pub struct Project {
    project: String,

    #[serde(rename = "ref")]
    reference: Option<String>,
    file: ProjectFile,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Local {
    local: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)] // This attribute allows for different structs in the same Vec
pub enum InputValue {
    Plain(String),
    Block(serde_yaml::Value),
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ComponentInclude {
    component: String,

    #[serde(default)]
    #[serde(deserialize_with = "deserialize_inputs")]
    inputs: Option<HashMap<String, InputValue>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Remote {
    remote: String,
}

// Custom deserializer for the `inputs` field
#[allow(clippy::unnecessary_wraps)]
fn deserialize_inputs<'de, D>(
    deserializer: D,
) -> Result<Option<HashMap<String, InputValue>>, D::Error>
where
    D: Deserializer<'de>,
{
    // Try to deserialize into a valid HashMap
    let result: Result<HashMap<String, InputValue>, D::Error> =
        Deserialize::deserialize(deserializer);

    match result {
        Ok(map) => Ok(Some(map)),
        Err(_) => Ok(None), // If deserialization fails, return None
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct LspConfiguration {
    #[serde(skip_serializing_if = "Option::is_none")]
    root_files: Option<Vec<String>>,
}

const DEFAULT_BRANCH_SUBFOLDER: &str = "default";
const MAX_CACHE_ITEMS: usize = 4;
