use std::{collections::HashMap, path::PathBuf, sync::Mutex, time::Instant};

use log::{debug, error, info};
use lsp_server::{Notification, Request};
use lsp_types::{
    request::GotoTypeDefinitionParams, CompletionParams, Diagnostic, DidChangeTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, DocumentDiagnosticParams, HoverParams,
    Url,
};

use crate::{
    parser::{self, ParserUtils},
    DefinitionResult, GitlabElement, HoverResult, LSPCompletion, LSPConfig, LSPLocation, LSPResult,
    ReferencesResult,
};

pub struct LSPHandlers {
    cfg: LSPConfig,
    store: Mutex<HashMap<String, String>>,
    nodes: Mutex<HashMap<String, HashMap<String, String>>>,
    stages: Mutex<HashMap<String, GitlabElement>>,
    variables: Mutex<HashMap<String, GitlabElement>>,
    indexing_in_progress: Mutex<bool>,
    parser: parser::Parser,
}

impl LSPHandlers {
    pub fn new(cfg: LSPConfig) -> LSPHandlers {
        let store = Mutex::new(HashMap::new());
        let nodes = Mutex::new(HashMap::new());
        let stages = Mutex::new(HashMap::new());
        let variables = Mutex::new(HashMap::new());
        let indexing_in_progress = Mutex::new(false);

        let events = LSPHandlers {
            cfg: cfg.clone(),
            store,
            nodes,
            stages,
            variables,
            indexing_in_progress,
            parser: parser::Parser::new(cfg.remote_urls, cfg.package_map, cfg.cache_path),
        };

        error!("IS IT ALWAYS NEW?");
        match events.index_workspace(events.cfg.root_dir.as_str()) {
            Ok(_) => {}
            Err(err) => {
                error!("error indexing workspace; err: {}", err);
            }
        };

        events
    }

    pub fn on_hover(&self, request: Request) -> Option<LSPResult> {
        let params = serde_json::from_value::<HoverParams>(request.params).ok()?;

        let store = self.store.lock().unwrap();
        let uri = &params.text_document_position_params.text_document.uri;
        let document = store.get::<String>(&uri.clone().into())?;

        let position = params.text_document_position_params.position;
        let line = document.lines().nth(position.line as usize)?;

        let word =
            ParserUtils::extract_word(line, position.character as usize)?.trim_end_matches(':');

        let mut hover = String::new();
        for (content_uri, content) in store.iter() {
            if let Some(element) = ParserUtils::get_root_node(content_uri, content, word) {
                // Check if we found the same line that triggered the hover event and discard it
                // adding format : because yaml parser removes it from the key
                if content_uri.ends_with(uri.as_str())
                    && line.eq(&format!("{}:", element.key.as_str()))
                {
                    continue;
                }

                if !hover.is_empty() {
                    hover = format!("{}\r\n--------\r\n", hover);
                }

                hover = format!("{}{}", hover, element.content?);
            }
        }

        if hover.is_empty() {
            return None;
        }

        hover = format!("```yaml \r\n{}\r\n```", hover);

        Some(LSPResult::Hover(HoverResult {
            id: request.id,
            content: hover,
        }))
    }

    pub fn on_change(&self, notification: Notification) -> Option<LSPResult> {
        let start = Instant::now();
        let params =
            serde_json::from_value::<DidChangeTextDocumentParams>(notification.params).ok()?;

        if params.content_changes.len() != 1 {
            return None;
        }

        // TODO: nodes

        let mut store = self.store.lock().unwrap();
        let mut all_nodes = self.nodes.lock().unwrap();
        // reset previous
        all_nodes.insert(params.text_document.uri.to_string(), HashMap::new());

        let mut all_variables = self.variables.lock().unwrap();

        if let Some(results) = self.parser.parse_contents(
            &params.text_document.uri,
            &params.content_changes.first()?.text,
            false,
        ) {
            for file in results.files {
                store.insert(file.path, file.content);
            }

            for node in results.nodes {
                info!("found node: {:?}", &node);
                all_nodes
                    .entry(node.uri)
                    .or_default()
                    .insert(node.key, node.description);
            }

            if !results.stages.is_empty() {
                let mut all_stages = self.stages.lock().unwrap();
                all_stages.clear();

                for stage in results.stages {
                    info!("found stage: {:?}", &stage);
                    all_stages.insert(stage.key.clone(), stage);
                }
            }

            // should be per file...
            // TODO: clear correct variables
            for variable in results.variables {
                info!("found variable: {:?}", &variable);
                all_variables.insert(variable.key.clone(), variable);
            }
        }

        error!("ONCHANGE ELAPSED: {:?}", start.elapsed());
        debug!("finished searching");

        None
    }

    pub fn on_open(&self, notification: Notification) -> Option<LSPResult> {
        let in_progress = self.indexing_in_progress.lock().unwrap();
        drop(in_progress);

        let params =
            serde_json::from_value::<DidOpenTextDocumentParams>(notification.params).ok()?;

        error!("ON_OPEN: {:?}", params.text_document.uri);

        debug!("started searching");

        let mut store = self.store.lock().unwrap();
        let mut all_nodes = self.nodes.lock().unwrap();
        let mut all_stages = self.stages.lock().unwrap();

        if let Some(results) =
            self.parser
                .parse_contents(&params.text_document.uri, &params.text_document.text, true)
        {
            for file in results.files {
                store.insert(file.path, file.content);
            }

            for node in results.nodes {
                info!("found node: {:?}", &node);

                all_nodes
                    .entry(node.uri)
                    .or_default()
                    .insert(node.key, node.description);
            }

            for stage in results.stages {
                info!("found stage: {:?}", &stage);
                all_stages.insert(stage.key.clone(), stage);
            }
        }

        debug!("finished searching");

        None
    }

    pub fn on_definition(&self, request: Request) -> Option<LSPResult> {
        let params = serde_json::from_value::<GotoTypeDefinitionParams>(request.params).ok()?;

        let store = self.store.lock().unwrap();
        let document_uri = params.text_document_position_params.text_document.uri;
        let document = store.get::<String>(&document_uri.clone().into())?;
        let position = params.text_document_position_params.position;

        let position_type = ParserUtils::get_position_type(document, position);
        match position_type {
            parser::CompletionType::RootNode | parser::CompletionType::Extend => {}
            _ => {
                error!("invalid position type for goto def");
                return None;
            }
        };

        let line = document.lines().nth(position.line as usize)?;
        let word =
            ParserUtils::extract_word(line, position.character as usize)?.trim_end_matches(':');

        let mut locations: Vec<LSPLocation> = vec![];

        for (uri, content) in store.iter() {
            if let Some(element) = ParserUtils::get_root_node(uri, content, word) {
                error!("ELEMENT: {:?}", element);
                if document_uri.as_str().ends_with(uri)
                    && line.eq(&format!("{}:", element.key.as_str()))
                {
                    continue;
                }

                locations.push(LSPLocation {
                    uri: uri.clone(),
                    range: element.range,
                });
            }
        }

        Some(LSPResult::Definition(DefinitionResult {
            id: request.id,
            locations,
        }))
    }

    pub fn on_completion(&self, request: Request) -> Option<LSPResult> {
        error!("GOT AUTOCOMPLETE REQ");
        let start = Instant::now();
        let params: CompletionParams = serde_json::from_value(request.params).ok()?;

        let store = self.store.lock().unwrap();
        let document_uri = params.text_document_position.text_document.uri;
        let document = store.get::<String>(&document_uri.clone().into())?;

        let position = params.text_document_position.position;
        let line = document.lines().nth(position.line as usize)?;

        let mut items: Vec<LSPCompletion> = vec![];

        let completion_type = ParserUtils::get_position_type(document, position);

        match completion_type {
            parser::CompletionType::None => return None,
            parser::CompletionType::RootNode => {}
            parser::CompletionType::Stage => {
                error!("STAGE");

                let stages = self.stages.lock().unwrap();
                let word = ParserUtils::word_before_cursor(
                    line,
                    position.character as usize,
                    |c: char| c.is_whitespace(),
                );
                let after = ParserUtils::word_after_cursor(line, position.character as usize);

                error!("stage: {:?}", stages.keys());
                error!("word: {:?}", word);

                for (stage, _) in stages.iter() {
                    if stage.contains(word) {
                        items.push(LSPCompletion {
                            label: stage.clone(),
                            details: None,
                            location: LSPLocation {
                                range: crate::Range {
                                    start: crate::LSPPosition {
                                        line: position.line,
                                        character: position.character - word.len() as u32,
                                    },
                                    end: crate::LSPPosition {
                                        line: position.line,
                                        character: position.character + after.len() as u32,
                                    },
                                },
                                ..Default::default()
                            },
                        })
                    }
                }
            }
            parser::CompletionType::Extend => {
                error!("EXTEND");

                let nodes = self.nodes.lock().unwrap();
                let word = ParserUtils::word_before_cursor(
                    line,
                    position.character as usize,
                    |c: char| c.is_whitespace(),
                );

                let after = ParserUtils::word_after_cursor(line, position.character as usize);

                for (_, node) in nodes.iter() {
                    for (node_key, node_description) in node.iter() {
                        if node_key.starts_with('.') && node_key.contains(word) {
                            items.push(LSPCompletion {
                                label: node_key.clone(),
                                details: Some(node_description.clone()),
                                location: LSPLocation {
                                    range: crate::Range {
                                        start: crate::LSPPosition {
                                            line: position.line,
                                            character: position.character - word.len() as u32,
                                        },
                                        end: crate::LSPPosition {
                                            line: position.line,
                                            character: position.character + after.len() as u32,
                                        },
                                    },
                                    ..Default::default()
                                },
                            })
                        }
                    }
                }
            }
            parser::CompletionType::Variable => {
                error!("VARIABLE");

                let variables = self.variables.lock().unwrap();
                let word = ParserUtils::word_before_cursor(
                    line,
                    position.character as usize,
                    |c: char| c == '$',
                );

                let after = ParserUtils::word_after_cursor(line, position.character as usize);

                for (variable, _) in variables.iter() {
                    if variable.starts_with(word) {
                        items.push(LSPCompletion {
                            label: variable.clone(),
                            details: None,
                            location: LSPLocation {
                                range: crate::Range {
                                    start: crate::LSPPosition {
                                        line: position.line,
                                        character: position.character - word.len() as u32,
                                    },
                                    end: crate::LSPPosition {
                                        line: position.line,
                                        character: position.character + after.len() as u32,
                                    },
                                },
                                ..Default::default()
                            },
                        })
                    }
                }
            }
        }

        error!("AUTOCOMPLETE ELAPSED: {:?}", start.elapsed());
        Some(LSPResult::Completion(crate::CompletionResult {
            id: request.id,
            list: items,
        }))
    }

    fn index_workspace(&self, root_dir: &str) -> anyhow::Result<()> {
        error!("LOCKING => INDEXING");

        let mut in_progress = self.indexing_in_progress.lock().unwrap();
        *in_progress = true;

        let start = Instant::now();

        let mut store = self.store.lock().unwrap();
        let mut all_nodes = self.nodes.lock().unwrap();
        let mut all_stages = self.stages.lock().unwrap();
        let mut all_variables = self.variables.lock().unwrap();

        let mut uri = Url::parse(format!("file://{}/", root_dir).as_str())?;
        info!("uri: {}", &uri);

        let list = std::fs::read_dir(root_dir)?;
        let mut root_file: Option<PathBuf> = None;

        for item in list.flatten() {
            if item.file_name() == ".gitlab-ci.yaml" || item.file_name() == ".gitlab-ci.yml" {
                root_file = Some(item.path());
                break;
            }
        }

        let root_file_content = match root_file {
            Some(root_file) => {
                let file_name = root_file.file_name().unwrap().to_str().unwrap();
                uri = uri.join(file_name)?;

                std::fs::read_to_string(root_file)?
            }
            _ => {
                return Err(anyhow::anyhow!("root file missing"));
            }
        };

        info!("URI: {}", &uri);
        if let Some(results) = self.parser.parse_contents(&uri, &root_file_content, true) {
            for file in results.files {
                info!("found file: {:?}", &file);
                store.insert(file.path, file.content);
            }

            for node in results.nodes {
                info!("found node: {:?}", &node);
                all_nodes
                    .entry(node.uri)
                    .or_default()
                    .insert(node.key, node.description);
            }

            for stage in results.stages {
                info!("found stage: {:?}", &stage);
                all_stages.insert(stage.key.clone(), stage);
            }

            for variable in results.variables {
                info!("found variable: {:?}", &variable);
                all_variables.insert(variable.key.clone(), variable);
            }
        }

        error!("ELAPSED: {:?}", start.elapsed());

        Ok(())
    }

    pub fn on_save(&self, notification: Notification) -> Option<LSPResult> {
        let _params =
            serde_json::from_value::<DidSaveTextDocumentParams>(notification.params).ok()?;

        // PUBLISH DIAGNOSTICS

        None
    }

    pub fn on_diagnostic(&self, request: Request) -> Option<LSPResult> {
        let start = Instant::now();
        let params = serde_json::from_value::<DocumentDiagnosticParams>(request.params).ok()?;
        let store = self.store.lock().unwrap();
        let all_nodes = self.nodes.lock().unwrap();

        let content: String = store
            .get(&params.text_document.uri.to_string())?
            .to_string();

        let extends = ParserUtils::get_all_extends(
            params.text_document.uri.to_string(),
            content.as_str(),
            None,
        );

        let mut diagnostics: Vec<Diagnostic> = vec![];

        'extend: for extend in extends {
            if extend.uri == params.text_document.uri.to_string() {
                for (_, root_nodes) in all_nodes.iter() {
                    if root_nodes.get(&extend.key).is_some() {
                        continue 'extend;
                    }
                }

                diagnostics.push(Diagnostic::new_simple(
                    lsp_types::Range {
                        start: lsp_types::Position {
                            line: extend.range.start.line,
                            character: extend.range.start.character,
                        },
                        end: lsp_types::Position {
                            line: extend.range.end.line,
                            character: extend.range.end.character,
                        },
                    },
                    format!("Rule: {} does not exist.", extend.key),
                ));
            }
        }

        let stages =
            ParserUtils::get_all_stages(params.text_document.uri.to_string(), content.as_str());
        error!("STAGES: {:?}", stages);

        let all_stages = self.stages.lock().unwrap();
        for stage in stages {
            if all_stages.get(&stage.key).is_none() {
                diagnostics.push(Diagnostic::new_simple(
                    lsp_types::Range {
                        start: lsp_types::Position {
                            line: stage.range.start.line,
                            character: stage.range.start.character,
                        },
                        end: lsp_types::Position {
                            line: stage.range.end.line,
                            character: stage.range.end.character,
                        },
                    },
                    format!("Stage: {} does not exist.", stage.key),
                ));
            }
        }

        error!("DIAGNOSTICS ELAPSED: {:?}", start.elapsed());
        Some(LSPResult::Diagnostics(crate::DiagnosticsResult {
            id: request.id,
            diagnostics,
        }))
    }

    pub fn on_references(&self, request: Request) -> Option<LSPResult> {
        let start = Instant::now();

        let params = serde_json::from_value::<lsp_types::ReferenceParams>(request.params).ok()?;

        let store = self.store.lock().unwrap();
        let document_uri = &params.text_document_position.text_document.uri;
        let document = store.get::<String>(&document_uri.clone().into())?;

        let position = params.text_document_position.position;
        let line = document.lines().nth(position.line as usize)?;

        let position_type = ParserUtils::get_position_type(document, position);
        let mut references: Vec<GitlabElement> = vec![];

        match position_type {
            parser::CompletionType::Extend => {
                let word = ParserUtils::extract_word(line, position.character as usize)?;

                for (uri, content) in store.iter() {
                    let mut extends =
                        ParserUtils::get_all_extends(uri.to_string(), content.as_str(), Some(word));
                    references.append(&mut extends);
                }
            }
            parser::CompletionType::RootNode => {
                let word = ParserUtils::extract_word(line, position.character as usize)?
                    .trim_end_matches(':');

                // currently support only those that are extends
                if word.starts_with('.') {
                    for (uri, content) in store.iter() {
                        let mut extends = ParserUtils::get_all_extends(
                            uri.to_string(),
                            content.as_str(),
                            Some(word),
                        );
                        references.append(&mut extends);
                    }
                }
            }
            _ => {}
        }

        error!("REFERENCES ELAPSED: {:?}", start.elapsed());

        Some(LSPResult::References(ReferencesResult {
            id: request.id,
            locations: references,
        }))
    }
}
