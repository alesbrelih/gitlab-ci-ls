//! End-to-end tests for CI/CD components running the real language server handlers
//! against the example project in `examples/components`.
//!
//! The example only uses components of the current project (`$CI_PROJECT_PATH`), so the
//! tests don't need network access.

use std::collections::HashMap;

use lsp_server::{Notification, Request, RequestId};
use lsp_types::{
    CompletionParams, DidChangeTextDocumentParams, DidOpenTextDocumentParams, GotoDefinitionParams,
    HoverParams, PartialResultParams, Position, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams, Url,
    VersionedTextDocumentIdentifier, WorkDoneProgressParams,
};
use tempfile::TempDir;

use super::{
    fs_utils::FSUtilsImpl, handlers::LSPHandlers, LSPConfig, LSPExperimental, LSPLocation,
    LSPResult,
};

const ROOT_FILE: &str = ".gitlab-ci.yml";
const DEPLOY_TEMPLATE: &str = "templates/deploy.yml";
const LINT_TEMPLATE: &str = "templates/lint/template.yml";
const INVALID_INPUTS: &str = "errors/invalid-inputs.yml";
const INVALID_TEMPLATE: &str = "errors/invalid-template.yml";

struct TestServer {
    handlers: LSPHandlers,
    // keeps the cache directory alive for the duration of the test
    _cache: TempDir,
}

fn example_root() -> String {
    format!("{}/examples/components", env!("CARGO_MANIFEST_DIR"))
}

fn uri(relative: &str) -> Url {
    Url::from_file_path(format!("{}/{relative}", example_root())).unwrap()
}

fn read(relative: &str) -> String {
    std::fs::read_to_string(format!("{}/{relative}", example_root())).unwrap()
}

/// Position of the `nth` occurrence of `needle` shifted by `offset` characters.
fn position(content: &str, needle: &str, nth: usize, offset: u32) -> Position {
    let (line, character) = content
        .lines()
        .enumerate()
        .flat_map(|(line, text)| text.match_indices(needle).map(move |(idx, _)| (line, idx)))
        .nth(nth)
        .unwrap_or_else(|| panic!("could not find occurrence {nth} of {needle:?}"));

    Position {
        line: u32::try_from(line).unwrap(),
        character: u32::try_from(character).unwrap() + offset,
    }
}

/// Position right after the last character of the line containing `needle`.
fn end_of_line(content: &str, needle: &str) -> Position {
    let (line, text) = content
        .lines()
        .enumerate()
        .find(|(_, text)| text.contains(needle))
        .unwrap_or_else(|| panic!("could not find line with {needle:?}"));

    Position {
        line: u32::try_from(line).unwrap(),
        character: u32::try_from(text.len()).unwrap(),
    }
}

fn text_position(uri: &Url, position: Position) -> TextDocumentPositionParams {
    TextDocumentPositionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        position,
    }
}

impl TestServer {
    fn new() -> Self {
        let cache = tempfile::tempdir().unwrap();
        let cache_path = format!("{}/", cache.path().display());
        std::fs::create_dir_all(format!("{cache_path}base")).unwrap();

        let handlers = LSPHandlers::new(
            LSPConfig {
                root_dir: example_root(),
                cache_path,
                package_map: HashMap::new(),
                remote_urls: vec![],
                variables: HashMap::new(),
                experimental: LSPExperimental {
                    dependencies_autocomplete_stage_filtering: false,
                },
            },
            Box::new(FSUtilsImpl::new(String::new())),
        );

        Self {
            handlers,
            _cache: cache,
        }
    }

    /// Opens the file and returns the reported diagnostics as `(line, message)`.
    fn open(&self, relative: &str) -> Vec<(u32, String)> {
        self.open_with_content(&uri(relative), &read(relative))
    }

    fn open_with_content(&self, uri: &Url, text: &str) -> Vec<(u32, String)> {
        let result = self.handlers.on_open(Notification::new(
            "textDocument/didOpen".to_string(),
            DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: "yaml".to_string(),
                    version: 1,
                    text: text.to_string(),
                },
            },
        ));

        let Some(LSPResult::Diagnostics(diagnostics)) = result else {
            panic!("expected diagnostics, got: {result:?}");
        };

        diagnostics
            .diagnostics
            .into_iter()
            .map(|d| (d.range.start.line, d.message))
            .collect()
    }

    /// Simulates the user editing a document.
    fn change(&self, uri: &Url, text: &str) {
        self.handlers.on_change(Notification::new(
            "textDocument/didChange".to_string(),
            DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: uri.clone(),
                    version: 2,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: text.to_string(),
                }],
            },
        ));
    }

    fn hover(&self, uri: &Url, position: Position) -> Option<String> {
        let result = self.handlers.on_hover(Request::new(
            RequestId::from(1),
            "textDocument/hover".to_string(),
            HoverParams {
                text_document_position_params: text_position(uri, position),
                work_done_progress_params: WorkDoneProgressParams::default(),
            },
        ));

        match result {
            Some(LSPResult::Hover(hover)) => Some(hover.content),
            None => None,
            other => panic!("expected hover, got: {other:?}"),
        }
    }

    fn completion(&self, uri: &Url, position: Position) -> Vec<String> {
        let result = self.handlers.on_completion(Request::new(
            RequestId::from(1),
            "textDocument/completion".to_string(),
            CompletionParams {
                text_document_position: text_position(uri, position),
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
                context: None,
            },
        ));

        match result {
            Some(LSPResult::Completion(completion)) => {
                completion.list.into_iter().map(|c| c.label).collect()
            }
            None => vec![],
            other => panic!("expected completion, got: {other:?}"),
        }
    }

    fn definition(&self, uri: &Url, position: Position) -> Vec<LSPLocation> {
        let result = self.handlers.on_definition(Request::new(
            RequestId::from(1),
            "textDocument/definition".to_string(),
            GotoDefinitionParams {
                text_document_position_params: text_position(uri, position),
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        ));

        match result {
            Some(LSPResult::Definition(definition)) => definition.locations,
            other => panic!("expected definition, got: {other:?}"),
        }
    }
}

fn assert_diagnostic(diagnostics: &[(u32, String)], line: u32, message: &str) {
    assert!(
        diagnostics
            .iter()
            .any(|(l, m)| *l == line && m.contains(message)),
        "expected diagnostic {message:?} on line {line}, got: {diagnostics:#?}"
    );
}

#[test]
fn test_valid_example_has_no_diagnostics() {
    let server = TestServer::new();

    for file in [ROOT_FILE, DEPLOY_TEMPLATE, LINT_TEMPLATE] {
        let diagnostics = server.open(file);
        assert!(
            diagnostics.is_empty(),
            "expected no diagnostics in {file}, got: {diagnostics:#?}"
        );
    }
}

#[test]
fn test_invalid_inputs_diagnostics() {
    let server = TestServer::new();
    let content = read(INVALID_INPUTS);
    let line = |needle: &str| position(&content, needle, 0, 0).line;

    let diagnostics = server.open(INVALID_INPUTS);

    assert_diagnostic(
        &diagnostics,
        line("environment: qa"),
        "Value needs to be one of: 'staging, production'",
    );
    assert_diagnostic(
        &diagnostics,
        line("version: 1.2.3"),
        "Value needs to match the pattern",
    );
    assert_diagnostic(
        &diagnostics,
        line("replicas: three"),
        "Expected a value of type `number`",
    );
    assert_diagnostic(
        &diagnostics,
        line("dry_run: \"true\""),
        "Expected a value of type `boolean`",
    );
    assert_diagnostic(&diagnostics, line("unknown: value"), "Invalid input key");

    // second include only sets `stage`
    let second_deploy = position(&content, "deploy@", 1, 0).line;
    assert_diagnostic(
        &diagnostics,
        second_deploy,
        "Missing required inputs: environment, version.",
    );

    assert_diagnostic(
        &diagnostics,
        line("fix: true"),
        "Component does not accept any inputs",
    );
    assert_diagnostic(
        &diagnostics,
        line("missing@"),
        "Could not resolve component",
    );

    // `image` has `rules`, so it isn't required; the first include sets everything else
    let first_deploy = position(&content, "deploy@", 0, 0).line;
    assert!(
        !diagnostics.iter().any(|(l, _)| *l == first_deploy),
        "{diagnostics:#?}"
    );
    assert_eq!(diagnostics.len(), 8, "{diagnostics:#?}");
}

#[test]
fn test_invalid_template_diagnostics() {
    let server = TestServer::new();
    let content = read(INVALID_TEMPLATE);
    let line = |needle: &str| position(&content, needle, 0, 0).line;

    let diagnostics = server.open(INVALID_TEMPLATE);

    assert_diagnostic(
        &diagnostics,
        line("inputs.typo"),
        "Input `typo` is not defined in `spec:inputs`.",
    );
    assert_diagnostic(
        &diagnostics,
        line("component.name"),
        "`component.name` needs to be declared in `spec:component`.",
    );
    assert_diagnostic(
        &diagnostics,
        line("to_upper"),
        "Unknown interpolation function `to_upper`",
    );
    assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
}

#[test]
fn test_hover_component_overview() {
    let server = TestServer::new();
    let content = read(ROOT_FILE);

    let hover = server
        .hover(&uri(ROOT_FILE), position(&content, "deploy@", 0, 2))
        .unwrap();

    assert!(hover.contains("## Component `deploy`"), "{hover}");
    assert!(hover.contains("templates/deploy.yml"), "{hover}");
    assert!(
        hover.contains("| `environment` | string | **required** | Environment to deploy to. |"),
        "{hover}"
    );
    assert!(
        hover.contains("| `replicas` | number | `1` | Number of application replicas. |"),
        "{hover}"
    );
    assert!(hover.contains("| `extra_args` | array | `[]` |"), "{hover}");
    assert!(
        hover.contains("| `image` | string | _from rules_ |"),
        "{hover}"
    );
}

#[test]
fn test_hover_component_without_inputs() {
    let server = TestServer::new();
    let content = read(ROOT_FILE);

    let hover = server
        .hover(&uri(ROOT_FILE), position(&content, "lint@", 0, 1))
        .unwrap();

    assert!(hover.contains("## Component `lint`"), "{hover}");
    assert!(hover.contains("This component has no inputs."), "{hover}");
}

#[test]
fn test_hover_component_input() {
    let server = TestServer::new();
    let content = read(ROOT_FILE);

    let hover = server
        .hover(&uri(ROOT_FILE), position(&content, "replicas:", 0, 2))
        .unwrap();

    assert!(hover.starts_with("### Input `replicas`"), "{hover}");
    assert!(hover.contains("number"), "{hover}");
    assert!(hover.contains("Number of application replicas."), "{hover}");
}

#[test]
fn test_hover_interpolation_in_template() {
    let server = TestServer::new();
    server.open(DEPLOY_TEMPLATE);
    let content = read(DEPLOY_TEMPLATE);
    let template = uri(DEPLOY_TEMPLATE);

    let hover = server
        .hover(&template, position(&content, "inputs.replicas", 0, 8))
        .unwrap();
    assert!(hover.starts_with("### Input `replicas`"), "{hover}");
    assert!(hover.contains("## Default:\n1"), "{hover}");

    let hover = server
        .hover(&template, position(&content, "component.version", 0, 12))
        .unwrap();
    assert!(hover.starts_with("### `component.version`"), "{hover}");
}

fn assert_same_position(actual: &super::LSPPosition, expected: Position) {
    assert_eq!(
        (actual.line, actual.character),
        (expected.line, expected.character)
    );
}

#[test]
fn test_definition_of_component_and_input() {
    let server = TestServer::new();
    let content = read(ROOT_FILE);
    let template_content = read(DEPLOY_TEMPLATE);

    let locations = server.definition(&uri(ROOT_FILE), position(&content, "deploy@", 0, 2));
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].uri, uri(DEPLOY_TEMPLATE).to_string());
    assert_eq!(locations[0].range.start.line, 0);

    let locations = server.definition(&uri(ROOT_FILE), position(&content, "replicas:", 0, 2));
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].uri, uri(DEPLOY_TEMPLATE).to_string());
    assert_same_position(
        &locations[0].range.start,
        position(&template_content, "replicas:", 0, 0),
    );
}

#[test]
fn test_definition_of_interpolated_input() {
    let server = TestServer::new();
    server.open(DEPLOY_TEMPLATE);
    let content = read(DEPLOY_TEMPLATE);
    let template = uri(DEPLOY_TEMPLATE);

    let locations = server.definition(&template, position(&content, "inputs.dry_run", 0, 9));

    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].uri, template.to_string());
    assert_same_position(
        &locations[0].range.start,
        position(&content, "dry_run:", 0, 0),
    );
}

#[test]
fn test_completion_of_input_keys() {
    let server = TestServer::new();
    let document = uri("completion.yml");
    // a half typed key is only recoverable by tree-sitter when something follows the include
    let partial_key_before_job = "include:
  - component: $CI_SERVER_FQDN/$CI_PROJECT_PATH/deploy@$CI_COMMIT_SHA
    inputs:
      environment: staging
      re

job:
  script: echo
";
    let partial_key_before_include = "include:
  - component: $CI_SERVER_FQDN/$CI_PROJECT_PATH/deploy@$CI_COMMIT_SHA
    inputs:
      environment: staging
      re
  - local: other.yml
";

    for content in [partial_key_before_job, partial_key_before_include] {
        server.change(&document, content);

        let labels = server.completion(&document, end_of_line(content, "      re"));

        assert_eq!(labels, vec!["replicas"], "{content}");
    }

    // inputs are matched by substring and already set ones aren't suggested again
    let content = "include:
  - component: $CI_SERVER_FQDN/$CI_PROJECT_PATH/deploy@$CI_COMMIT_SHA
    inputs:
      environment: staging
      r:
";
    server.change(&document, content);
    let mut labels = server.completion(&document, position(content, "r:", 0, 1));
    labels.sort();
    assert_eq!(labels, vec!["dry_run", "extra_args", "replicas", "version"]);
}

#[test]
fn test_completion_of_input_values() {
    let server = TestServer::new();
    let document = uri("completion.yml");
    let content = "include:
  - component: $CI_SERVER_FQDN/$CI_PROJECT_PATH/deploy@$CI_COMMIT_SHA
    inputs:
      environment:
      dry_run:
      replicas:
";
    server.change(&document, content);

    // options
    let labels = server.completion(&document, end_of_line(content, "environment:"));
    assert_eq!(labels, vec!["staging", "production"]);

    // booleans
    let labels = server.completion(&document, end_of_line(content, "dry_run:"));
    assert_eq!(labels, vec!["true", "false"]);

    // nothing to suggest for free-form values
    let labels = server.completion(&document, end_of_line(content, "replicas:"));
    assert!(labels.is_empty(), "{labels:?}");

    // values are filtered by what was typed
    let content = content.replace("environment:\n", "environment: prod\n");
    server.change(&document, &content);
    let labels = server.completion(&document, end_of_line(&content, "environment:"));
    assert_eq!(labels, vec!["production"]);
}

#[test]
fn test_completion_inside_interpolation() {
    let server = TestServer::new();
    let template = uri(DEPLOY_TEMPLATE);
    let original = read(DEPLOY_TEMPLATE);

    let complete = |line: &str| {
        let content = format!("{original}    - echo {line}\n");
        server.change(&template, &content);
        server.completion(&template, end_of_line(&content, &format!("echo {line}")))
    };

    assert_eq!(complete("$[[ "), vec!["inputs", "component"]);
    assert_eq!(complete("$[[ comp"), vec!["component"]);
    assert_eq!(
        complete("$[[ inputs."),
        vec![
            "stage",
            "environment",
            "version",
            "replicas",
            "dry_run",
            "extra_args",
            "image"
        ]
    );
    assert_eq!(complete("$[[ inputs.re"), vec!["replicas"]);
    assert_eq!(
        complete("$[[ component."),
        vec!["name", "sha", "version", "reference"]
    );
    assert_eq!(
        complete("$[[ inputs.stage | "),
        vec!["expand_vars", "truncate", "posix_escape"]
    );
    assert_eq!(complete("$[[ inputs.stage | tr"), vec!["truncate"]);
}
