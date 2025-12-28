use std::{collections::HashMap, fs, path::PathBuf, sync::{Arc, Mutex}, time::Instant};

use anyhow::anyhow;
use log::{debug, error, info, warn};
use lsp_server::{Notification, Request};
use lsp_types::{
    request::GotoTypeDefinitionParams, CompletionParams, Diagnostic, DidChangeTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, HoverParams, Position, RenameParams,
    TextDocumentPositionParams, TextEdit, Url,
};
use regex::Regex;

use crate::gitlab_ci_ls_parser::{
    parser_utils::ParserUtils, workspace::{Workspace, IndexingState}, DiagnosticsNotification, LspConfiguration, NodeDefinition,
    PrepareRenameResult, RenameResult, DEFAULT_BRANCH_SUBFOLDER, MAX_CACHE_ITEMS,
};

use super::{
    fs_utils,
    parser::{self, PositionType},
    parser_utils, treesitter, CompletionResult, Component, ComponentInput, DefinitionResult,
    GitlabElement, GitlabFileElements, GitlabInputElement, HoverResult, IncludeInformation,
    LSPCompletion, LSPConfig, LSPLocation, LSPPosition, LSPResult, Range, ReferencesResult,
    RemoteInclude, RuleReference,
};

#[allow(clippy::module_name_repetitions)]
pub struct LSPHandlers {
    cfg: LSPConfig,
    workspaces: Mutex<Vec<Arc<Workspace>>>,
    parser: Box<dyn parser::Parser>,
}

impl LSPHandlers {
    pub fn new(cfg: LSPConfig, fs_utils: Box<dyn fs_utils::FSUtils>) -> LSPHandlers {
        let workspaces = Mutex::new(vec![]);

        let events = LSPHandlers {
            cfg: cfg.clone(),
            workspaces,
            parser: Box::new(parser::ParserImpl::new(
                cfg.remote_urls,
                cfg.package_map,
                cfg.cache_path,
                Box::new(treesitter::TreesitterImpl::new()),
                fs_utils,
            )),
        };

        if let Err(err) = events.index_workspace(events.cfg.root_dir.as_str()) {
            error!("error indexing workspace; err: {err}");
        }

        events
    }

    fn get_workspace_for_uri(&self, uri: &str) -> Option<Arc<Workspace>> {
        let workspaces = self.workspaces.lock().unwrap();
        workspaces
            .iter()
            .find(|w| w.files_included.lock().unwrap().contains(uri))
            .cloned()
            .or_else(|| workspaces.first().cloned())
    }

    fn default_stages() -> Vec<String> {
        vec![
            ".pre".to_string(),
            "build".to_string(),
            "test".to_string(),
            "deploy".to_string(),
            ".post".to_string(),
        ]
    }

    // When renaming or some other action that will be handled later on we need
    // to prevent modifications on cached/downloaded files.
    fn can_path_be_modified(&self, path: &str) -> bool {
        !path
            .to_lowercase()
            .contains(&self.cfg.cache_path.to_lowercase())
    }

    #[allow(clippy::too_many_lines)]
    pub fn on_hover(&self, request: Request) -> Option<LSPResult> {
        let params = serde_json::from_value::<HoverParams>(request.params).ok()?;
        let uri = &params.text_document_position_params.text_document.uri;

        let workspace = self.get_workspace_for_uri(uri.as_str())?;

        let store = workspace.store.lock().unwrap();
        let node_list = workspace.nodes_ordered_list.lock().unwrap();
        let nodes = workspace.nodes.lock().unwrap();

        let document = store.get::<String>(&uri.to_string())?;

        let position = params.text_document_position_params.position;
        let line = document.lines().nth(position.line as usize)?;

        let word = parser_utils::ParserUtils::extract_word(line, position.character as usize)?
            .trim_end_matches(':');

        match self.parser.get_position_type(document, position) {
            parser::PositionType::Extend | PositionType::Dependency => {
                for (document_uri, node) in nodes.iter() {
                    for (key, element) in node {
                        if key.eq(word) {
                            let cnt = match self.parser.get_full_definition(
                                GitlabElement {
                                    key: key.clone(),
                                    content: element.content.clone(),
                                    uri: document_uri.clone(),
                                    ..Default::default()
                                },
                                &node_list,
                            ) {
                                Ok(c) => c,
                                Err(err) => return Some(LSPResult::Error(err)),
                            };

                            return Some(LSPResult::Hover(HoverResult {
                                id: request.id,
                                content: format!("```yaml\n{cnt}\n```"),
                            }));
                        }
                    }
                }

                None
            }
            parser::PositionType::RootNode => {
                let document_uri = format!("file://{}", uri.path());
                let node = nodes.get(&document_uri)?;

                for (key, element) in node {
                    if key.eq(word) {
                        let cnt = match self.parser.get_full_definition(
                            GitlabElement {
                                key: key.clone(),
                                content: element.content.clone(),
                                uri: document_uri.clone(),
                                ..Default::default()
                            },
                            &node_list,
                        ) {
                            Ok(c) => c,
                            Err(err) => return Some(LSPResult::Error(err)),
                        };

                        return Some(LSPResult::Hover(HoverResult {
                            id: request.id,
                            content: format!("```yaml\n{cnt}\n```"),
                        }));
                    }
                }

                None
            }
            parser::PositionType::RuleReference(RuleReference { node }) => {
                for (document_uri, n) in nodes.iter() {
                    for (key, element) in n {
                        if key.eq(&node) {
                            let cnt = match self.parser.get_full_definition(
                                GitlabElement {
                                    key: key.clone(),
                                    content: element.content.clone(),
                                    uri: document_uri.clone(),
                                    ..Default::default()
                                },
                                &node_list,
                            ) {
                                Ok(c) => c,
                                Err(err) => return Some(LSPResult::Error(err)),
                            };

                            return Some(LSPResult::Hover(HoverResult {
                                id: request.id,
                                content: format!("```yaml\n{cnt}\n```"),
                            }));
                        }
                    }
                }

                None
            }

            parser::PositionType::Needs(NodeDefinition { name }) => {
                let need_split = ParserUtils::strip_quotes(&name)
                    .split(' ')
                    .collect::<Vec<&str>>();
                let node_name = need_split.first()?;

                for (document_uri, n) in nodes.iter() {
                    for (key, element) in n {
                        if key.eq(node_name) {
                            let cnt = match self.parser.get_full_definition(
                                GitlabElement {
                                    key: key.clone(),
                                    content: element.content.clone(),
                                    uri: document_uri.clone(),
                                    ..Default::default()
                                },
                                &node_list,
                            ) {
                                Ok(c) => c,
                                Err(err) => return Some(LSPResult::Error(err)),
                            };

                            return Some(LSPResult::Hover(HoverResult {
                                id: request.id,
                                content: format!("```yaml\n{cnt}\n```"),
                            }));
                        }
                    }
                }

                None
            }
            _ => None,
        }
    }

    pub fn on_change(&self, notification: Notification) -> Option<LSPResult> {
        let start = Instant::now();
        let params =
            serde_json::from_value::<DidChangeTextDocumentParams>(notification.params).ok()?;

        if params.content_changes.len() != 1 {
            return None;
        }

        let workspace = self.get_workspace_for_uri(params.text_document.uri.as_str())?;

        let mut store = workspace.store.lock().unwrap();
        let mut all_nodes = workspace.nodes.lock().unwrap();
        let mut all_nodes_ordered_list = workspace.nodes_ordered_list.lock().unwrap();
        let mut all_stages_ordered_list = workspace.stages_ordered_list.lock().unwrap();
        // reset previous
        all_nodes.insert(params.text_document.uri.to_string(), HashMap::new());

        let mut all_variables = workspace.variables.lock().unwrap();

        let mut all_components = workspace.components.lock().unwrap();

        let u = workspace.root_uri.clone();

        if let Some(results) = self.parser.parse_contents(
            &params.text_document.uri,
            &params.content_changes.first()?.text,
            &u,
            false,
        ) {
            for file in results.files {
                workspace.files_included.lock().unwrap().insert(file.path.clone());
                store.insert(file.path, file.content);
            }

            for node in results.nodes.clone() {
                all_nodes
                    .entry(node.uri.clone())
                    .or_default()
                    .insert(node.key.clone(), node);
            }

            if let Some(e) = all_nodes_ordered_list
                .iter_mut()
                .find(|e| e.uri == params.text_document.uri.to_string())
            {
                e.elements.clone_from(&results.nodes);
            } else {
                // new file?
                all_nodes_ordered_list.push(GitlabFileElements {
                    uri: params.text_document.uri.to_string(),
                    elements: results.nodes.clone(),
                });
            }

            if !results.stages.is_empty() {
                let mut all_stages = workspace.stages.lock().unwrap();
                all_stages.clear();

                for stage in &results.stages {
                    info!("found stage: {:?}", &stage);
                    all_stages.insert(stage.key.clone(), stage.clone());
                }

                all_stages_ordered_list.clone_from(
                    &results
                        .stages
                        .into_iter()
                        .map(|s| s.key)
                        .collect::<Vec<String>>(),
                );
            }

            // should be per file...
            // TODO: clear correct variables
            for variable in results.variables {
                info!("found variable: {:?}", &variable);
                all_variables.insert(variable.key.clone(), variable);
            }

            for component in results.components {
                info!("found component: {:?}", &component);
                all_components.insert(component.uri.clone(), component);
            }
        }

        info!("ONCHANGE ELAPSED: {:?}", start.elapsed());

        None
    }

    pub fn on_open(&self, notification: Notification) -> Option<LSPResult> {
        let params =
            serde_json::from_value::<DidOpenTextDocumentParams>(notification.params).ok()?;

        let workspace = self.get_workspace_for_uri(params.text_document.uri.as_str())?;

        let in_progress = workspace.indexing_state.lock().unwrap();
        drop(in_progress);

        let mut store = workspace.store.lock().unwrap();
        let mut all_nodes = workspace.nodes.lock().unwrap();
        let mut all_stages = workspace.stages.lock().unwrap();

        let u = workspace.root_uri.clone();

        if let Some(results) = self.parser.parse_contents(
            &params.text_document.uri,
            &params.text_document.text,
            &u,
            true,
        ) {
            for file in results.files {
                workspace.files_included.lock().unwrap().insert(file.path.clone());
                store.insert(file.path, file.content);
            }

            for node in results.nodes {
                info!("found node: {:?}", &node);

                all_nodes
                    .entry(node.uri.clone())
                    .or_default()
                    .insert(node.key.clone(), node);
            }

            for stage in results.stages {
                info!("found stage: {:?}", &stage);
                all_stages.insert(stage.key.clone(), stage);
            }
        }

        info!("finished searching");

        // releasing lock because generate diagnostics grabs it
        // and is used in two places
        drop(store);
        drop(all_nodes);
        drop(all_stages);

        self.generate_diagnostics(params.text_document.uri)
    }

    #[allow(clippy::too_many_lines)]
    pub fn on_definition(&self, request: Request) -> Option<LSPResult> {
        let params = serde_json::from_value::<GotoTypeDefinitionParams>(request.params).ok()?;
        let document_uri = params.text_document_position_params.text_document.uri;

        let workspace = self.get_workspace_for_uri(document_uri.as_str())?;

        let store = workspace.store.lock().unwrap();
        let store = &*store;
        let node_list = workspace.nodes_ordered_list.lock().unwrap();
        let document = store.get::<String>(&document_uri.to_string())?;
        let position = params.text_document_position_params.position;
        let stages = workspace.stages.lock().unwrap();

        let mut locations: Vec<LSPLocation> = vec![];

        match self.parser.get_position_type(document, position) {
            parser::PositionType::RootNode
            | parser::PositionType::Extend
            | parser::PositionType::Dependency => {
                let line = document.lines().nth(position.line as usize)?;
                let word =
                    parser_utils::ParserUtils::extract_word(line, position.character as usize)?
                        .trim_end_matches(':');

                for (uri, content) in store {
                    if let Some(element) = self.parser.get_root_node(uri, content, word) {
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
            }
            parser::PositionType::Include(info) => {
                if let Some(include) = self.on_definition_include(&workspace, info, store) {
                    locations.push(include);
                }
            }
            parser::PositionType::Needs(node) => {
                for (uri, content) in store {
                    let need_split = node.name.split(' ').collect::<Vec<&str>>();

                    // need can be: needs: "wow [matrix1, matrix2]
                    // currently just ignore matrixes
                    match need_split.len() {
                        1 => {
                            if let Some(element) = self.parser.get_root_node(
                                uri,
                                content,
                                parser_utils::ParserUtils::strip_quotes(node.name.as_str()),
                            ) {
                                locations.push(LSPLocation {
                                    uri: uri.clone(),
                                    range: element.range,
                                });
                            }
                        }

                        2 => {
                            if let Some(element) = self.parser.get_root_node(
                                uri,
                                content,
                                parser_utils::ParserUtils::strip_quotes(need_split[0]),
                            ) {
                                locations.push(LSPLocation {
                                    uri: uri.clone(),
                                    range: element.range,
                                });
                            }
                        }

                        invalid => {
                            warn!(
                                "gotoref: invalid split len. got: {invalid}; needs: {need_split:?}"
                            );
                        }
                    }
                }
            }
            parser::PositionType::Stage => {
                let line = document.lines().nth(position.line as usize)?;
                let word =
                    parser_utils::ParserUtils::extract_word(line, position.character as usize)?;

                if let Some(el) = stages.get(word) {
                    locations.push(LSPLocation {
                        uri: el.uri.clone(),
                        range: el.range.clone(),
                    });
                }
            }
            parser::PositionType::Variable => {
                let line = document.lines().nth(position.line as usize)?;
                let word =
                    parser_utils::ParserUtils::extract_variable(line, position.character as usize)?;

                let variable_locations = self.parser.get_variable_definitions(
                    word,
                    document_uri.as_str(),
                    position,
                    store,
                    &node_list,
                )?;

                for location in variable_locations {
                    locations.push(LSPLocation {
                        uri: location.uri,
                        range: location.range,
                    });
                }
                let mut root = workspace
                    .variables
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(name, _)| name.starts_with(word))
                    .map(|(_, el)| LSPLocation {
                        uri: el.uri.clone(),
                        range: el.range.clone(),
                    })
                    .collect::<Vec<LSPLocation>>();

                locations.append(&mut root);
            }
            parser::PositionType::RuleReference(RuleReference { node }) => {
                for (uri, content) in store {
                    if let Some(element) = self.parser.get_root_node(uri, content, &node) {
                        locations.push(LSPLocation {
                            uri: uri.clone(),
                            range: element.range,
                        });
                    }
                }
            }
            parser::PositionType::None => {
                error!("invalid position type for goto def");
                return None;
            }
        }

        Some(LSPResult::Definition(DefinitionResult {
            id: request.id,
            locations,
        }))
    }

    #[allow(clippy::too_many_lines)]
    fn on_definition_include(
        &self,
        workspace: &Workspace,
        info: IncludeInformation,
        store: &HashMap<String, String>,
    ) -> Option<LSPLocation> {
        match info {
            IncludeInformation {
                local: Some(local),
                remote: None,
                remote_url: None,
                basic: None,
                component: None,
            } => {
                let local = parser_utils::ParserUtils::strip_quotes(&local.path);

                LSPHandlers::on_definition_local(local, store)
            }
            IncludeInformation {
                local: None,
                remote: Some(remote),
                remote_url: None,
                basic: None,
                component: None,
            } => {
                let file = remote.file?;
                let file = parser_utils::ParserUtils::strip_quotes(&file).trim_start_matches('/');

                let path = if let Some(reference) = remote.reference {
                    format!("{}/{}/{}", remote.project?, reference, file)
                } else {
                    format!("{}/{}/{}", remote.project?, DEFAULT_BRANCH_SUBFOLDER, file)
                };

                store
                    .keys()
                    .find(|uri| uri.ends_with(&path))
                    .map(|uri| LSPLocation {
                        uri: uri.clone(),
                        range: Range {
                            start: LSPPosition {
                                line: 0,
                                character: 0,
                            },
                            end: LSPPosition {
                                line: 0,
                                character: 0,
                            },
                        },
                    })
            }
            IncludeInformation {
                local: None,
                remote: None,
                remote_url: None,
                basic: None,
                component: Some(component),
            } => {
                let component_uri = component
                    .uri
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();

                workspace.components
                    .lock()
                    .unwrap()
                    .values()
                    .find(|c| c.uri == component_uri)
                    .map(|c| LSPLocation {
                        uri: c.local_path.clone(),
                        range: Range {
                            start: LSPPosition {
                                line: 0,
                                character: 0,
                            },
                            end: LSPPosition {
                                line: 0,
                                character: 0,
                            },
                        },
                    })
            }
            IncludeInformation {
                local: None,
                remote: None,
                remote_url: Some(remote_url),
                basic: None,
                component: None,
            } => {
                let remote_url = parser_utils::ParserUtils::strip_quotes(remote_url.path.as_str());
                LSPHandlers::on_definition_remote(remote_url, store)
            }
            IncludeInformation {
                local: None,
                remote: None,
                remote_url: None,
                basic: Some(basic_url),
                component: None,
            } => {
                let url = parser_utils::ParserUtils::strip_quotes(&basic_url.path);
                if let Ok(url) = Url::parse(url) {
                    LSPHandlers::on_definition_remote(url.as_str(), store)
                } else {
                    LSPHandlers::on_definition_local(url, store)
                }
            }
            _ => None,
        }
    }

    pub fn on_definition_local(
        local_url: &str,
        store: &HashMap<String, String>,
    ) -> Option<LSPLocation> {
        let local_url = local_url.trim_start_matches('.');

        store
            .keys()
            .find(|uri| uri.ends_with(local_url))
            .map(|uri| LSPLocation {
                uri: uri.clone(),
                range: Range {
                    start: LSPPosition {
                        line: 0,
                        character: 0,
                    },
                    end: LSPPosition {
                        line: 0,
                        character: 0,
                    },
                },
            })
    }

    pub fn on_definition_remote(
        remote_url: &str,
        store: &HashMap<String, String>,
    ) -> Option<LSPLocation> {
        let path_hash = parser_utils::ParserUtils::remote_path_to_hash(remote_url);

        store
            .keys()
            .find(|uri| uri.ends_with(format!("_{path_hash}.yaml").as_str()))
            .map(|uri| LSPLocation {
                uri: uri.clone(),
                range: Range {
                    start: LSPPosition {
                        line: 0,
                        character: 0,
                    },
                    end: LSPPosition {
                        line: 0,
                        character: 0,
                    },
                },
            })
    }

    pub fn on_completion(&self, request: Request) -> Option<LSPResult> {
        let start = Instant::now();
        let params: CompletionParams = serde_json::from_value(request.params).ok()?;
        let document_uri = params.text_document_position.text_document.uri;

        let workspace = self.get_workspace_for_uri(document_uri.as_str())?;

        let store = workspace.store.lock().unwrap();
        let document = store.get::<String>(&document_uri.clone().into())?;

        let position = params.text_document_position.position;
        let line = document.lines().nth(position.line as usize)?;

        let items = match self.parser.get_position_type(document, position) {
            parser::PositionType::Stage => {
                self.on_completion_stages(&workspace, line, position).ok()?
            }
            parser::PositionType::Dependency => self
                .on_completion_dependencies(
                    &workspace,
                    document_uri.as_ref(),
                    document,
                    line,
                    position,
                )
                .ok()?,
            parser::PositionType::Extend => {
                self.on_completion_extends(&workspace, line, position).ok()?
            }
            parser::PositionType::Variable => {
                self.on_completion_variables(&workspace, line, position).ok()?
            }
            parser::PositionType::Needs(_) => {
                self.on_completion_needs(&workspace, line, position).ok()?
            }
            parser::PositionType::Include(IncludeInformation {
                remote: None,
                remote_url: None,
                local: None,
                basic: None,
                component: Some(component),
            }) => self
                .on_completion_component(&workspace, line, position, &component)
                .ok()?,
            parser::PositionType::Include(IncludeInformation {
                remote: Some(remote),
                remote_url: None,
                local: None,
                basic: None,
                component: None,
            }) => self
                .on_completion_remote(&workspace, line, position, &remote)
                .ok()?,
            parser::PositionType::RuleReference(_) => {
                self.on_completion_rule_reference(&workspace, line, position).ok()?
            }
            _ => return None,
        };

        info!("AUTOCOMPLETE ELAPSED: {:?}", start.elapsed());

        Some(LSPResult::Completion(CompletionResult {
            id: request.id,
            list: items,
        }))
    }

    fn on_completion_stages(
        &self,
        workspace: &Workspace,
        line: &str,
        position: Position,
    ) -> anyhow::Result<Vec<LSPCompletion>> {
        let stages = {
            let locked_stages = workspace
                .stages
                .lock()
                .map_err(|e| anyhow::anyhow!("failed to lock stages: {}", e))?;

            let keys: Vec<_> = locked_stages.keys().map(ToString::to_string).collect();

            if keys.is_empty() {
                LSPHandlers::default_stages()
            } else {
                keys
            }
        };

        let word = parser_utils::ParserUtils::word_before_cursor(
            line,
            position.character as usize,
            |c: char| c.is_whitespace(),
        );
        let after =
            parser_utils::ParserUtils::word_after_cursor(line, position.character as usize, |c| {
                c.is_whitespace()
            });

        let items = stages
            .iter()
            .filter(|stage| stage.contains(word))
            .flat_map(|stage| -> anyhow::Result<LSPCompletion> {
                Ok(LSPCompletion {
                    label: stage.clone(),
                    details: None,
                    location: LSPLocation {
                        range: Range {
                            start: LSPPosition {
                                line: position.line,
                                character: position.character - u32::try_from(word.len())?,
                            },
                            end: LSPPosition {
                                line: position.line,
                                character: position.character + u32::try_from(after.len())?,
                            },
                        },
                        ..Default::default()
                    },
                })
            })
            .collect();

        Ok(items)
    }

    fn on_completion_dependencies(
        &self,
        workspace: &Workspace,
        uri: &str,
        document: &str,
        line: &str,
        position: Position,
    ) -> anyhow::Result<Vec<LSPCompletion>> {
        let start = Instant::now();

        let nodes = workspace
            .nodes
            .lock()
            .map_err(|err| anyhow!("failed to lock nodes: {}", err))?;

        let word = parser_utils::ParserUtils::word_before_cursor(
            line,
            position.character as usize,
            |c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '[',
        );

        let after =
            parser_utils::ParserUtils::word_after_cursor(line, position.character as usize, |c| {
                c.is_whitespace() || c == '"' || c == '\'' || c == ']'
            });

        // autocomplete filtering by stage; experimental opt infeature due to longer responses ATM
        let all_nodes_ordered_list = workspace.nodes_ordered_list.lock().unwrap();
        let all_stages_ordered_list = workspace.stages_ordered_list.lock().unwrap();
        let mut previous_stages = HashMap::new();

        if self
            .cfg
            .experimental
            .dependencies_autocomplete_stage_filtering
        {
            if let Some(root_node) = self.parser.get_root_node_at_position(document, position) {
                if let Ok(full_definition) = self
                    .parser
                    .get_full_definition(root_node.clone(), &all_nodes_ordered_list)
                {
                    let stage = self.parser.get_all_stages(uri, &full_definition, None);
                    if let Some(stage) = stage.first() {
                        for s in all_stages_ordered_list.iter() {
                            previous_stages.insert(s.clone(), true);

                            if ParserUtils::strip_quotes(s) == ParserUtils::strip_quotes(&stage.key)
                            {
                                break;
                            }
                        }
                    }
                }
            }
        }

        let items = nodes
            .values()
            .flat_map(|needs| needs.iter())
            .filter(|(node_key, _)| !node_key.starts_with('.') && node_key.contains(word))
            .filter(|(_, element)| {
                if self
                    .cfg
                    .experimental
                    .dependencies_autocomplete_stage_filtering
                {
                    if previous_stages.keys().len() == 0 {
                        return true;
                    }

                    if let Some(content) = &element.content {
                        // check if stage is defined at top node
                        let stage = self.parser.get_all_stages(uri, content, None);
                        if let Some(s) = stage.first() {
                            return previous_stages.contains_key(&s.key);
                        } else if let Ok(full_definition) = self
                            .parser
                            .get_full_definition((*element).clone(), &all_nodes_ordered_list)
                        {
                            // stage isn't defined at top node, so we need to get full job definition
                            // and find stage
                            let stage = self.parser.get_all_stages(uri, &full_definition, None);
                            if let Some(stage) = stage.first() {
                                return previous_stages.contains_key(&stage.key);
                            }
                        }
                    }

                    true
                } else {
                    true
                }
            })
            .flat_map(|(node_key, element)| -> anyhow::Result<LSPCompletion> {
                Ok(LSPCompletion {
                    label: node_key.clone(),
                    details: Some(format!(
                        "```yaml\r\n{}\r\n```",
                        element.clone().content.unwrap_or(String::new())
                    )),
                    location: LSPLocation {
                        range: Range {
                            start: LSPPosition {
                                line: position.line,
                                character: position.character - u32::try_from(word.len())?,
                            },
                            end: LSPPosition {
                                line: position.line,
                                character: position.character + u32::try_from(after.len())?,
                            },
                        },
                        ..Default::default()
                    },
                })
            })
            .collect();

        info!("completion dependencies: {:?}", start.elapsed());

        Ok(items)
    }

    fn on_completion_extends(
        &self,
        workspace: &Workspace,
        line: &str,
        position: Position,
    ) -> anyhow::Result<Vec<LSPCompletion>> {
        let nodes = workspace
            .nodes
            .lock()
            .map_err(|e| anyhow!("failed to lock nodes: {}", e))?;

        let word = parser_utils::ParserUtils::word_before_cursor(
            line,
            position.character as usize,
            |c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '[',
        );

        let after = parser_utils::ParserUtils::word_after_cursor(
            line,
            position.character as usize,
            |c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ']',
        );

        let items = nodes
            .values()
            .flat_map(|n| n.iter())
            .filter(|(node_key, _)| node_key.starts_with('.') && node_key.contains(word))
            .flat_map(|(node_key, element)| -> anyhow::Result<LSPCompletion> {
                Ok(LSPCompletion {
                    label: node_key.clone(),
                    details: Some(format!(
                        "```yaml\r\n{}\r\n```",
                        element.clone().content.unwrap_or(String::new())
                    )),
                    location: LSPLocation {
                        range: Range {
                            start: LSPPosition {
                                line: position.line,
                                character: position
                                    .character
                                    .saturating_sub(u32::try_from(word.len())?),
                            },
                            end: LSPPosition {
                                line: position.line,
                                character: position.character + u32::try_from(after.len())?,
                            },
                        },
                        ..Default::default()
                    },
                })
            })
            .collect();

        Ok(items)
    }

    fn on_completion_variables(
        &self,
        workspace: &Workspace,
        line: &str,
        position: Position,
    ) -> anyhow::Result<Vec<LSPCompletion>> {
        let variables = workspace
            .variables
            .lock()
            .map_err(|e| anyhow!("failed to lock variables: {}", e))?;

        let word = parser_utils::ParserUtils::word_before_cursor(
            line,
            position.character as usize,
            |c: char| c == '$',
        );

        let after =
            parser_utils::ParserUtils::word_after_cursor(line, position.character as usize, |c| {
                c.is_whitespace() || c == '"' || c == '\''
            });

        let items = variables
            .keys()
            .filter(|v| v.starts_with(word))
            .flat_map(|v| -> anyhow::Result<LSPCompletion> {
                Ok(LSPCompletion {
                    label: v.clone(),
                    details: None,
                    location: LSPLocation {
                        range: Range {
                            start: LSPPosition {
                                line: position.line,
                                character: position.character - u32::try_from(word.len())?,
                            },
                            end: LSPPosition {
                                line: position.line,
                                character: position.character + u32::try_from(after.len())?,
                            },
                        },
                        ..Default::default()
                    },
                })
            })
            .collect();

        Ok(items)
    }

    fn on_completion_rule_reference(
        &self,
        workspace: &Workspace,
        line: &str,
        position: Position,
    ) -> anyhow::Result<Vec<LSPCompletion>> {
        let nodes = workspace
            .nodes
            .lock()
            .map_err(|err| anyhow!("failed to lock nodes: {}", err))?;

        let word = parser_utils::ParserUtils::word_before_cursor(
            line,
            position.character as usize,
            |c: char| c == '\'' || c == '"',
        );

        let after =
            parser_utils::ParserUtils::word_after_cursor(line, position.character as usize, |c| {
                c == '\'' || c == '"'
            });

        let items = nodes
            .values()
            .flat_map(|needs| needs.iter())
            .filter(|(node_key, _)| node_key.contains(word))
            .flat_map(|(node_key, element)| -> anyhow::Result<LSPCompletion> {
                Ok(LSPCompletion {
                    label: node_key.clone(),
                    details: Some(format!(
                        "```yaml\r\n{}\r\n```",
                        element.clone().content.unwrap_or(String::new())
                    )),
                    location: LSPLocation {
                        range: Range {
                            start: LSPPosition {
                                line: position.line,
                                character: position.character - u32::try_from(word.len())?,
                            },
                            end: LSPPosition {
                                line: position.line,
                                character: position.character + u32::try_from(after.len())?,
                            },
                        },
                        ..Default::default()
                    },
                })
            })
            .collect();

        Ok(items)
    }

    fn on_completion_needs(
        &self,
        workspace: &Workspace,
        line: &str,
        position: Position,
    ) -> anyhow::Result<Vec<LSPCompletion>> {
        let nodes = workspace
            .nodes
            .lock()
            .map_err(|err| anyhow!("failed to lock nodes: {}", err))?;
        let word = parser_utils::ParserUtils::word_before_cursor(
            line,
            position.character as usize,
            |c: char| c.is_whitespace(),
        );
        let after =
            parser_utils::ParserUtils::word_after_cursor(line, position.character as usize, |c| {
                c.is_whitespace()
            });

        let items = nodes
            .values()
            .flat_map(|needs| needs.iter())
            .filter(|(node_key, _)| !node_key.starts_with('.') && node_key.contains(word))
            .flat_map(|(node_key, element)| -> anyhow::Result<LSPCompletion> {
                Ok(LSPCompletion {
                    label: node_key.clone(),
                    details: Some(format!(
                        "```yaml\r\n{}\r\n```",
                        element.clone().content.unwrap_or(String::new())
                    )),
                    location: LSPLocation {
                        range: Range {
                            start: LSPPosition {
                                line: position.line,
                                character: position.character - u32::try_from(word.len())?,
                            },
                            end: LSPPosition {
                                line: position.line,
                                character: position.character + u32::try_from(after.len())?,
                            },
                        },
                        ..Default::default()
                    },
                })
            })
            .collect();

        Ok(items)
    }

    fn resolve_root_files(files: Vec<String>, project_root_dir: &Url) -> Vec<PathBuf> {
        let mut resolved_root_files = Vec::new();
        let project_root_path = project_root_dir.to_file_path().ok().unwrap_or_else(|| {
            warn!("Could not convert project_root_dir to PathBuf");
            PathBuf::new()
        });

        for file in files {
            let absolute_path = project_root_path.join(&file);
            if absolute_path.is_dir() {
                // Handle directories: find all .gitlab-ci.yml files recursively
                let pattern = format!("{}/**/*.gitlab-ci.yml", absolute_path.display());
                match glob::glob(&pattern) {
                    Ok(paths) => {
                        for path in paths.flatten() {
                            resolved_root_files.push(path);
                        }
                    }
                    Err(e) => warn!("Error globbing directory {pattern}: {e}"),
                }
            } else if file.contains('*') || file.contains('?') || file.contains('[') {
                // Handle glob patterns
                let pattern = absolute_path.display().to_string(); // Use absolute_path for glob pattern
                match glob::glob(&pattern) {
                    Ok(paths) => {
                        for path in paths.flatten() {
                            resolved_root_files.push(path);
                        }
                    }
                    Err(e) => warn!("Error globbing pattern {pattern}: {e}"),
                }
            } else if absolute_path.exists() {
                // Handle regular file paths
                resolved_root_files.push(absolute_path);
            }
        }
        resolved_root_files
    }

    #[allow(clippy::too_many_lines)]
    fn index_workspace(&self, root_dir: &str) -> anyhow::Result<()> {
        let start = Instant::now();

        info!("importing from root file");
        let project_root_dir = Url::parse(format!("file://{root_dir}/").as_str())?;
        info!("uri: {}", &project_root_dir);

        let list = std::fs::read_dir(root_dir)?;
        let mut root_files: Vec<PathBuf> = vec![];
        let mut config_file: Option<PathBuf> = None;

        for item in list.flatten() {
            if item.file_name() == ".gitlab-ci.yaml" || item.file_name() == ".gitlab-ci.yml" {
                root_files.push(item.path());
            }
            if item.file_name() == ".gitlab-ci-ls.yaml" || item.file_name() == ".gitlab-ci-ls.yml" {
                config_file = Some(item.path());
            }
        }

        // parse config file to struct
        if let Some(config) = config_file {
            let config_content = std::fs::read_to_string(&config)?;
            let lsp_config = match serde_yaml::from_str(&config_content) {
                Ok(c) => c,
                Err(err) => {
                    warn!("error parsing config file: {err}");
                    LspConfiguration::default()
                }
            };

            if let Some(files) = lsp_config.root_files {
                root_files = Self::resolve_root_files(files, &project_root_dir);
            }
        }

        if root_files.is_empty() {
            return Err(anyhow::anyhow!("root file missing"));
        }

        for root_file in root_files {
            let root_file_content = std::fs::read_to_string(&root_file)?;

            let file_uri = {
                let Some(root_file_str) = root_file.to_str() else {
                    // continue to next root file if its there
                    continue;
                };

                let root_file_str_with_schema = if root_file_str.starts_with('/') {
                    format!("file://{root_file_str}")
                } else {
                    root_file_str.to_string()
                };
                match Url::parse(&root_file_str_with_schema) {
                    Ok(u) => u,
                    Err(err) => {
                        warn!("could not parse root file: {root_file_str_with_schema}; got err: {err}");
                        continue;
                    }
                }
            };

            let workspace = Arc::new(Workspace::new(file_uri.clone()));
            {
                let mut ws_list = self.workspaces.lock().unwrap();
                ws_list.push(workspace.clone());
            }

            {
                let mut in_progress = workspace.indexing_state.lock().unwrap();
                *in_progress = IndexingState::InProgress;
            }

            let mut store = workspace.store.lock().unwrap();
            let mut all_nodes = workspace.nodes.lock().unwrap();
            let mut all_nodes_ordered_list = workspace.nodes_ordered_list.lock().unwrap();
            let mut all_stages_ordered_list = workspace.stages_ordered_list.lock().unwrap();
            let mut all_stages = workspace.stages.lock().unwrap();
            let mut all_variables = workspace.variables.lock().unwrap();
            let mut all_components = workspace.components.lock().unwrap();

            info!("importing files from base");
            let base_uri = format!("{}base", self.cfg.cache_path);
            let base_uri_path = Url::parse(format!("file://{base_uri}/").as_str())?;
            if let Ok(read_dir) = std::fs::read_dir(&base_uri) {
                for dir in read_dir.flatten() {
                    let file_uri = base_uri_path.join(dir.file_name().to_str().unwrap())?;
                    let file_content = std::fs::read_to_string(dir.path())?;

                    if let Some(results) =
                        self.parser
                            .parse_contents(&file_uri, &file_content, &project_root_dir, true)
                    {
                        for file in results.files {
                            info!("found file: {:?}", &file);
                            workspace.files_included.lock().unwrap().insert(file.path.clone());
                            store.insert(file.path, file.content);
                        }

                        for node in results.nodes {
                            info!("found node: {:?}", &node);

                            all_nodes
                                .entry(node.uri.clone())
                                .or_default()
                                .insert(node.key.clone(), node);
                        }

                        for stage in results.stages {
                            info!("found stage: {:?}", &stage);
                            all_stages.insert(stage.key.clone(), stage);
                        }

                        for variable in results.variables {
                            info!("found variable: {:?}", &variable);
                            all_variables.insert(variable.key.clone(), variable);
                        }

                        for component in results.components {
                            info!("found component: {:?}", &component);
                            all_components.insert(component.uri.clone(), component);
                        }
                    }
                }
            }

            if let Some(results) =
                self.parser
                    .parse_contents(&file_uri, &root_file_content, &project_root_dir, true)
            {
                for file in results.files {
                    info!("found file: {:?}", &file);
                    workspace.files_included.lock().unwrap().insert(file.path.clone());
                    store.insert(file.path, file.content);
                }

                for n in &results.nodes {
                    if let Some(el) = all_nodes_ordered_list.iter_mut().find(|e| e.uri == n.uri) {
                        el.elements.push(n.clone());
                    } else {
                        all_nodes_ordered_list.push(GitlabFileElements {
                            uri: n.uri.clone(),
                            elements: vec![n.clone()],
                        });
                    }
                }

                for node in results.nodes {
                    info!("found node: {:?}", &node);

                    all_nodes
                        .entry(node.uri.clone())
                        .or_default()
                        .insert(node.key.clone(), node);
                }

                for stage in &results.stages {
                    info!("found stage: {:?}", &stage);
                    all_stages.insert(stage.key.clone(), stage.clone());
                }

                all_stages_ordered_list.clone_from(
                    &results
                        .stages
                        .into_iter()
                        .map(|s| s.key)
                        .collect::<Vec<String>>(),
                );

                for variable in results.variables {
                    info!("found variable: {:?}", &variable);
                    all_variables.insert(variable.key.clone(), variable);
                }

                for component in results.components {
                    info!("found component: {:?}", &component);
                    all_components.insert(component.uri.clone(), component);
                }
            }

            {
                let mut in_progress = workspace.indexing_state.lock().unwrap();
                *in_progress = IndexingState::Completed;
            }
        }

        info!("INDEX WORKSPACE ELAPSED: {:?}", start.elapsed());

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn generate_diagnostics(&self, document_uri: lsp_types::Url) -> Option<LSPResult> {
        let start = Instant::now();
        let workspace = self.get_workspace_for_uri(document_uri.as_str())?;

        let store = workspace.store.lock().unwrap();
        let all_nodes = workspace.nodes.lock().unwrap();

        let content: String = store.get(&document_uri.to_string())?.clone();

        let extends = self
            .parser
            .get_all_extends(document_uri.to_string(), content.as_str(), None);

        let mut diagnostics: Vec<Diagnostic> = vec![];

        'extend: for extend in extends {
            if extend.uri == document_uri.to_string() {
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

        let stages = self
            .parser
            .get_all_stages(document_uri.as_ref(), content.as_str(), None);

        let all_stages = {
            let locked_stages = workspace.stages.lock().unwrap();

            let keys: Vec<_> = locked_stages.keys().map(ToString::to_string).collect();

            if keys.is_empty() {
                LSPHandlers::default_stages()
            } else {
                keys
            }
        };

        for stage in stages {
            if !all_stages.contains(&stage.key) {
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

        let needs = self
            .parser
            .get_all_job_needs(document_uri.to_string(), content.as_str(), None);

        'needs: for need in needs {
            let need_split = need.key.split(' ').collect::<Vec<&str>>();

            match need_split.len() {
                1 => {
                    // default needs containing just a reference
                    // to a job
                    for (_, node) in all_nodes.iter() {
                        if node.get(need.key.as_str()).is_some() {
                            continue 'needs;
                        }
                    }
                }

                2 => {
                    // special needs where it can reference a matrix inside a job
                    // needs: "job-name [matrix-value-1,matrix-value-2,..]
                    // currently just check split value that it matches a job
                    // TODO: handle matrix references

                    let node_key = need_split[0];
                    for (_, node) in all_nodes.iter() {
                        if node.get(node_key).is_some() {
                            continue 'needs;
                        }
                    }
                }

                invalid => {
                    warn!("invalid split len. got: {invalid}; needs: {need_split:?}");
                }
            }

            diagnostics.push(Diagnostic::new_simple(
                lsp_types::Range {
                    start: lsp_types::Position {
                        line: need.range.start.line,
                        character: need.range.start.character,
                    },
                    end: lsp_types::Position {
                        line: need.range.end.line,
                        character: need.range.end.character,
                    },
                },
                format!("Job: {} does not exist.", need.key),
            ));
        }

        let components = self
            .parser
            .get_all_components(document_uri.as_ref(), content.as_str());

        let all_components = workspace.components.lock().unwrap();
        for component in components {
            if let Some(spec) = all_components.get(&component.key) {
                component.inputs.iter().for_each(|i| {
                    // check invalid ones -> those that aren't defined in spec
                    let spec_definition = &spec.inputs.iter().find(|si| si.key == i.key);

                    if let Some(spec_definition) = spec_definition {
                        generate_component_diagnostics_from_spec(
                            i,
                            spec_definition,
                            &mut diagnostics,
                        );
                    } else {
                        // wasn't found in spec -> invalid key
                        diagnostics.push(Diagnostic::new_simple(
                            lsp_types::Range {
                                start: lsp_types::Position {
                                    line: i.range.start.line,
                                    character: i.range.start.character,
                                },
                                end: lsp_types::Position {
                                    line: i.range.end.line,
                                    character: i.range.end.character,
                                },
                            },
                            format!(
                                "Invalid input key. Key needs to be one of: '{}'.",
                                spec.inputs
                                    .iter()
                                    .map(|i| i.key.clone())
                                    .collect::<Vec<String>>()
                                    .join(", ")
                            ),
                        ));
                    }
                });
            }
        }

        let caches = self
            .parser
            .get_all_multi_caches(document_uri.as_ref(), content.as_str());

        let cache_diagnostics = caches.iter().flat_map(|c| c.cache_items.iter().skip(MAX_CACHE_ITEMS).map(|el| {
                Diagnostic::new_simple(
                    lsp_types::Range {
                        start: lsp_types::Position {
                            line: el.range.start.line,
                            character: el.range.start.character,
                        },
                        end: lsp_types::Position {
                            line: el.range.end.line,
                            character: el.range.end.character,
                        },
                    },
                    "You can have a maximum of 4 caches: https://docs.gitlab.com/ee/ci/caching/#use-multiple-caches".to_string(),
                )
            }));

        diagnostics.extend(cache_diagnostics);

        info!("DIAGNOSTICS ELAPSED: {:?}", start.elapsed());
        Some(LSPResult::Diagnostics(DiagnosticsNotification {
            uri: document_uri,
            diagnostics,
        }))
    }

    pub fn on_save(&self, notification: Notification) -> Option<LSPResult> {
        let params =
            serde_json::from_value::<DidSaveTextDocumentParams>(notification.params).ok()?;

        self.generate_diagnostics(params.text_document.uri)
    }

    pub fn on_references(&self, request: Request) -> Option<LSPResult> {
        let start = Instant::now();

        let params = serde_json::from_value::<lsp_types::ReferenceParams>(request.params).ok()?;
        let document_uri = &params.text_document_position.text_document.uri;

        let workspace = self.get_workspace_for_uri(document_uri.as_str())?;

        let store = workspace.store.lock().unwrap();
        let document = store.get::<String>(&document_uri.to_string())?;

        let position = params.text_document_position.position;
        let line = document.lines().nth(position.line as usize)?;

        let position_type = self.parser.get_position_type(document, position);
        let mut references: Vec<GitlabElement> = vec![];

        match position_type {
            parser::PositionType::Extend => {
                let word =
                    parser_utils::ParserUtils::extract_word(line, position.character as usize)?;

                for (uri, content) in store.iter() {
                    let mut extends =
                        self.parser
                            .get_all_extends(uri.clone(), content.as_str(), Some(word));
                    references.append(&mut extends);
                }
            }
            parser::PositionType::RootNode => {
                let word =
                    parser_utils::ParserUtils::extract_word(line, position.character as usize)?
                        .trim_end_matches(':');

                // currently support only those that are extends
                if word.starts_with('.') {
                    for (uri, content) in store.iter() {
                        let mut extends =
                            self.parser
                                .get_all_extends(uri.clone(), content.as_str(), Some(word));
                        references.append(&mut extends);
                    }
                } else {
                    for (uri, content) in store.iter() {
                        let mut extends = self.parser.get_all_job_needs(
                            uri.clone(),
                            content.as_str(),
                            Some(word),
                        );
                        references.append(&mut extends);
                    }
                }
            }
            parser::PositionType::Stage => {
                let word =
                    parser_utils::ParserUtils::extract_word(line, position.character as usize);

                for (uri, content) in store.iter() {
                    let mut stages = self.parser.get_all_stages(uri, content.as_str(), word);
                    references.append(&mut stages);
                }
            }
            _ => {}
        }

        info!("REFERENCES ELAPSED: {:?}", start.elapsed());

        Some(LSPResult::References(ReferencesResult {
            id: request.id,
            locations: references,
        }))
    }

    #[allow(clippy::unnecessary_wraps, clippy::too_many_lines)]
    fn on_completion_component(
        &self,
        workspace: &Workspace,
        line: &str,
        position: Position,
        component: &Component,
    ) -> anyhow::Result<Vec<LSPCompletion>> {
        if component.inputs.iter().any(|i| i.hovered) {
            let word = parser_utils::ParserUtils::word_before_cursor(
                line,
                position.character as usize,
                |c: char| c.is_whitespace(),
            );

            let after = parser_utils::ParserUtils::word_after_cursor(
                line,
                position.character as usize,
                |c| c.is_whitespace() || c == ':',
            );

            let components_store = workspace.components.lock().unwrap();
            let Some(component_spec) = components_store.get(&component.uri) else {
                warn!(
                    "could not find component spec; indexing went wrong!; searching for {}",
                    component.uri
                );

                return Ok(vec![]);
            };

            // filter out those that were already used
            let valid_input_autocompletes: Vec<super::ComponentInput> = component_spec
                .inputs
                .iter()
                .filter(|&i| !component.inputs.iter().any(|ci| ci.key == i.key))
                .cloned() // Clone each element to get an owned version
                .collect();

            let items = valid_input_autocompletes
                .into_iter()
                .filter(|i| i.key.contains(word))
                .flat_map(|i| -> anyhow::Result<LSPCompletion> {
                    Ok(LSPCompletion {
                        label: i.key.clone(),
                        details: Some(i.autocomplete_details()),
                        location: LSPLocation {
                            range: Range {
                                start: LSPPosition {
                                    line: position.line,
                                    character: position.character - u32::try_from(word.len())?,
                                },
                                end: LSPPosition {
                                    line: position.line,
                                    character: position.character + u32::try_from(after.len())?,
                                },
                            },
                            ..Default::default()
                        },
                    })
                })
                .collect();

            return Ok(items);
        } else if let Some(hovered_input) = component.inputs.iter().find(|i| i.value_plain.hovered)
        {
            let word = parser_utils::ParserUtils::word_before_cursor(
                line,
                position.character as usize,
                |c| c.is_whitespace() || c == ':',
            );

            let after = parser_utils::ParserUtils::word_after_cursor(
                line,
                position.character as usize,
                |c: char| c.is_whitespace(),
            );

            let components_store = workspace.components.lock().unwrap();
            let Some(component_spec) = components_store.get(&component.uri) else {
                warn!(
                    "could not find component spec; indexing went wrong!; searching for {}",
                    component.uri
                );

                return Ok(vec![]);
            };

            if let Some(input_spec) = component_spec
                .inputs
                .iter()
                .find(|i| i.key == hovered_input.key)
            {
                if let Some(options) = &input_spec.options {
                    let items = options
                        .iter()
                        .filter(|option| option.contains(word))
                        .flat_map(|option| -> anyhow::Result<LSPCompletion> {
                            Ok(LSPCompletion {
                                label: option.clone(),
                                details: None,
                                location: LSPLocation {
                                    range: Range {
                                        start: LSPPosition {
                                            line: position.line,
                                            character: position.character
                                                - u32::try_from(word.len())?,
                                        },
                                        end: LSPPosition {
                                            line: position.line,
                                            character: position.character
                                                + u32::try_from(after.len())?,
                                        },
                                    },
                                    ..Default::default()
                                },
                            })
                        })
                        .collect();

                    return Ok(items);
                }
            }
        }

        Ok(vec![])
    }

    #[allow(clippy::too_many_lines)]
    pub fn on_prepare_rename(&self, request: Request) -> Option<LSPResult> {
        let start = Instant::now();
        let params: TextDocumentPositionParams = serde_json::from_value(request.params).ok()?;

        let document_uri = params.text_document.uri;

        if !self.can_path_be_modified(document_uri.as_ref()) {
            return Some(LSPResult::PrepareRename(super::PrepareRenameResult {
                id: request.id,
                range: None,
                err: Some("Cannot rename externally included files".to_string()),
            }));
        }

        let workspace = self.get_workspace_for_uri(document_uri.as_str())?;

        let store = workspace.store.lock().unwrap();
        let document = store.get::<String>(&document_uri.clone().into())?;

        let position = params.position;
        let line = document.lines().nth(position.line as usize)?;

        let res = match self.parser.get_position_type(document, position) {
            parser::PositionType::RootNode => {
                let word = parser_utils::ParserUtils::word_before_cursor(
                    line,
                    position.character as usize,
                    char::is_whitespace,
                );
                let after = parser_utils::ParserUtils::word_after_cursor(
                    line,
                    position.character as usize,
                    char::is_whitespace,
                )
                .trim_end_matches(':');

                let full_word = format!("{word}{after}");
                if LSPHandlers::is_predefined_root_element(&full_word) {
                    return Some(LSPResult::PrepareRename(super::PrepareRenameResult {
                        id: request.id,
                        range: None,
                        err: Some("Cannot rename Gitlab elements".to_string()),
                    }));
                }

                Some(LSPResult::PrepareRename(super::PrepareRenameResult {
                    id: request.id,
                    range: Some(Range {
                        start: LSPPosition {
                            line: position.line,
                            character: position.character - u32::try_from(word.len()).ok()?,
                        },
                        end: LSPPosition {
                            line: position.line,
                            character: position.character + u32::try_from(after.len()).ok()?,
                        },
                    }),
                    err: None,
                }))
            }
            parser::PositionType::Extend
            | parser::PositionType::Needs(_)
            | parser::PositionType::RuleReference(_) => {
                let word = parser_utils::ParserUtils::word_before_cursor(
                    line,
                    position.character as usize,
                    |c| c.is_whitespace() || c == '\'' || c == '"',
                );
                let after = parser_utils::ParserUtils::word_after_cursor(
                    line,
                    position.character as usize,
                    |c| c.is_whitespace() || c == '\'' || c == '"',
                );

                let job = format!("{word}{after}");
                for (uri, content) in store.iter() {
                    if !self.can_path_be_modified(uri) {
                        continue;
                    }

                    if self.parser.get_root_node_key(uri, content, &job).is_some() {
                        return Some(LSPResult::PrepareRename(PrepareRenameResult {
                            id: request.id,
                            range: Some(Range {
                                start: LSPPosition {
                                    line: position.line,
                                    character: position.character
                                        - u32::try_from(word.len()).ok()?,
                                },
                                end: LSPPosition {
                                    line: position.line,
                                    character: position.character
                                        + u32::try_from(after.len()).ok()?,
                                },
                            }),
                            err: None,
                        }));
                    }
                }
                return Some(LSPResult::PrepareRename(super::PrepareRenameResult {
                    id: request.id,
                    range: None,
                    err: Some("Could not find definition".to_string()),
                }));
            }
            _ => Some(LSPResult::PrepareRename(super::PrepareRenameResult {
                id: request.id,
                range: None,
                err: Some("Not supported".to_string()),
            })),
        };

        info!("ON PREPARE RENAME ELAPSED: {:?}", start.elapsed());

        res
    }

    #[allow(clippy::too_many_lines)]
    pub fn on_rename(&self, request: Request) -> Option<LSPResult> {
        let start = Instant::now();
        let params: RenameParams = serde_json::from_value(request.params).ok()?;

        info!("got rename params: {params:?}");

        let document_uri = params.text_document_position.text_document.uri;

        // This is redundant but I guess could be needed for when prepare_rename isn't supported
        // by the client
        if !self.can_path_be_modified(document_uri.as_ref()) {
            return Some(LSPResult::Rename(super::RenameResult {
                id: request.id,
                edits: None,
                err: Some("Cannot rename externally included files".to_string()),
            }));
        }

        let workspace = self.get_workspace_for_uri(document_uri.as_str())?;

        let store = workspace.store.lock().unwrap();
        let document = store.get::<String>(&document_uri.clone().into())?;

        let position = params.text_document_position.position;
        let line = document.lines().nth(position.line as usize)?;

        let mut edits: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        match self.parser.get_position_type(document, position) {
            parser::PositionType::RootNode => {
                let text_edits = edits.entry(document_uri.clone()).or_default();

                let word = parser_utils::ParserUtils::word_before_cursor(
                    line,
                    position.character as usize,
                    char::is_whitespace,
                );
                let after = parser_utils::ParserUtils::word_after_cursor(
                    line,
                    position.character as usize,
                    char::is_whitespace,
                )
                .trim_end_matches(':');

                let full_word = format!("{word}{after}");

                if LSPHandlers::is_predefined_root_element(&full_word) {
                    return Some(LSPResult::Rename(super::RenameResult {
                        id: request.id,
                        edits: None,
                        err: Some("Cannot rename Gitlab elements".to_string()),
                    }));
                }

                text_edits.push(TextEdit {
                    new_text: params.new_name.clone(),
                    range: lsp_types::Range {
                        start: Position {
                            line: position.line,
                            character: position.character - u32::try_from(word.len()).ok()?,
                        },
                        end: Position {
                            line: position.line,
                            character: position.character + u32::try_from(after.len()).ok()?,
                        },
                    },
                });

                for (uri, content) in store.iter() {
                    if !self.can_path_be_modified(uri) {
                        continue;
                    }

                    // TODO: ? should be removed and just skip this entry
                    let text_edits = edits.entry(Url::parse(uri).ok()?).or_default();

                    text_edits.append(&mut self.rename_extends(
                        uri,
                        content,
                        &full_word,
                        &params.new_name,
                    ));

                    text_edits.append(&mut self.rename_needs(
                        uri,
                        content,
                        &full_word,
                        &params.new_name,
                    ));

                    text_edits.append(&mut self.rename_rule_references(
                        uri,
                        content,
                        &full_word,
                        &params.new_name,
                    ));
                }
            }
            parser::PositionType::Extend
            | parser::PositionType::RuleReference(_)
            | parser::PositionType::Needs(_) => {
                let word = parser_utils::ParserUtils::word_before_cursor(
                    line,
                    position.character as usize,
                    |c| c.is_whitespace() || c == '\'' || c == '"',
                );

                let after = parser_utils::ParserUtils::word_after_cursor(
                    line,
                    position.character as usize,
                    |c| c.is_whitespace() || c == '\'' || c == '"',
                );

                let job = format!("{word}{after}");

                let mut is_renamed_job_inside_the_project = false;

                for (uri, content) in store.iter() {
                    if !self.can_path_be_modified(uri) {
                        continue;
                    }

                    // TODO: ? should be removed and just skip this entry
                    let text_edits = edits.entry(Url::parse(uri).ok()?).or_default();

                    if let Some(r) = self.rename_root_node(uri, content, &job, &params.new_name) {
                        is_renamed_job_inside_the_project = true;
                        text_edits.push(r);
                    }

                    text_edits.append(&mut self.rename_extends(
                        uri,
                        content,
                        &job,
                        &params.new_name,
                    ));

                    text_edits.append(&mut self.rename_needs(uri, content, &job, &params.new_name));

                    text_edits.append(&mut self.rename_rule_references(
                        uri,
                        content,
                        &job,
                        &params.new_name,
                    ));
                }

                // adding this at the bottom because if we are trying to rename some extend that
                // was declared only in cached files this wont be reached
                if !is_renamed_job_inside_the_project {
                    return Some(LSPResult::Rename(super::RenameResult {
                        id: request.id,
                        edits: None,
                        err: Some(
                            "Cannot rename extend which has definition outside project scope"
                                .to_string(),
                        ),
                    }));
                }
            }
            _ => {
                warn!("invalid type for rename");
            }
        }

        info!("ON RENAME ELAPSED: {:?}", start.elapsed());

        Some(LSPResult::Rename(RenameResult {
            id: request.id,
            edits: Some(edits),
            err: None,
        }))
    }

    fn rename_extends(
        &self,
        uri: &str,
        content: &str,
        current_name: &str,
        new_name: &str,
    ) -> Vec<TextEdit> {
        let extends = self
            .parser
            .get_all_extends(uri.to_string(), content, Some(current_name));

        let mut text_edits = vec![];
        for e in extends {
            text_edits.push(TextEdit {
                range: lsp_types::Range {
                    start: Position {
                        line: e.range.start.line,
                        character: e.range.start.character,
                    },
                    end: Position {
                        line: e.range.end.line,
                        character: e.range.end.character,
                    },
                },
                new_text: new_name.to_string(),
            });
        }

        text_edits
    }

    fn rename_needs(
        &self,
        uri: &str,
        content: &str,
        current_name: &str,
        new_name: &str,
    ) -> Vec<TextEdit> {
        let extends = self
            .parser
            .get_all_job_needs(uri.to_string(), content, Some(current_name));

        let mut text_edits = vec![];
        for e in extends {
            text_edits.push(TextEdit {
                range: lsp_types::Range {
                    start: Position {
                        line: e.range.start.line,
                        character: e.range.start.character,
                    },
                    end: Position {
                        line: e.range.end.line,
                        character: e.range.end.character,
                    },
                },
                new_text: new_name.to_string(),
            });
        }

        text_edits
    }

    fn rename_rule_references(
        &self,
        uri: &str,
        content: &str,
        full_word: &str,
        new_name: &str,
    ) -> Vec<TextEdit> {
        let rule_references =
            self.parser
                .get_all_rule_references(uri.to_string(), content, Some(full_word));

        let mut text_edits = vec![];
        for r in rule_references {
            text_edits.push(TextEdit {
                range: lsp_types::Range {
                    start: Position {
                        line: r.range.start.line,
                        character: r.range.start.character,
                    },
                    end: Position {
                        line: r.range.end.line,
                        character: r.range.end.character,
                    },
                },
                new_text: new_name.to_string(),
            });
        }

        text_edits
    }

    fn is_predefined_root_element(full_word: &str) -> bool {
        let predefined = ["default", "variables", "include", "stages", "image"];
        predefined.contains(&full_word)
    }

    fn rename_root_node(
        &self,
        uri: &str,
        content: &str,
        current_name: &str,
        new_name: &str,
    ) -> Option<TextEdit> {
        if let Some(e) = self.parser.get_root_node_key(uri, content, current_name) {
            return Some(TextEdit {
                range: lsp_types::Range {
                    start: Position {
                        line: e.range.start.line,
                        character: e.range.start.character,
                    },
                    end: Position {
                        line: e.range.end.line,
                        character: e.range.end.character,
                    },
                },
                new_text: new_name.to_string(),
            });
        }

        None
    }

    fn on_completion_remote(
        &self,
        _workspace: &Workspace,
        line: &str,
        position: Position,
        remote: &RemoteInclude,
    ) -> anyhow::Result<Vec<LSPCompletion>> {
        let Some(project) = &remote.project else {
            return Ok(vec![]);
        };

        let word = parser_utils::ParserUtils::word_before_cursor(
            line,
            position.character as usize,
            |c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '/' || c == '\\',
        );

        let after =
            parser_utils::ParserUtils::word_after_cursor(line, position.character as usize, |c| {
                c.is_whitespace() || c == '"' || c == '\'' || c == '/' || c == '\\'
            });

        let path = if let Some(reference) = &remote.reference {
            format!("{project}/{reference}/")
        } else {
            format!("{project}/{DEFAULT_BRANCH_SUBFOLDER}/")
        };

        let (current, previous) =
            ParserUtils::find_path_at_cursor(line, usize::try_from(position.character).unwrap());

        let cache = &self.cfg.cache_path;
        let full_path = format!("{cache}{path}{previous}");

        let mut lsp_completions = vec![];
        for entry in fs::read_dir(full_path)? {
            let entry = entry?;
            let path = entry.path();

            let path_str = path.file_name().unwrap().to_string_lossy();

            if path_str.starts_with('.') {
                debug!("path starts with .; skipping");
                continue;
            }

            if !current.trim().is_empty() && !path_str.contains(&current) {
                debug!("path: {path_str:?} doesnt contain: {current:?}");
                continue;
            }

            if path.is_file() && !path_str.ends_with(".yaml") && !path_str.ends_with(".yml") {
                continue;
            }

            let c = LSPCompletion {
                label: path_str.to_string(),
                details: None,
                location: LSPLocation {
                    range: Range {
                        start: LSPPosition {
                            line: position.line,
                            character: position.character - u32::try_from(word.len())?,
                        },
                        end: LSPPosition {
                            line: position.line,
                            character: position.character + u32::try_from(after.len())?,
                        },
                    },
                    ..Default::default()
                },
            };

            lsp_completions.push(c);
        }

        Ok(lsp_completions)
    }
}

fn generate_component_diagnostics_from_spec(
    i: &GitlabInputElement,
    spec_definition: &ComponentInput,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(input_value_element) = &i.value_plain {
        if let Some(input_value) = &input_value_element.content {
            // check options
            if let Some(options) = &spec_definition.options {
                if !options.contains(input_value) {
                    diagnostics.push(Diagnostic::new_simple(
                        lsp_types::Range {
                            start: lsp_types::Position {
                                line: input_value_element.range.start.line,
                                character: input_value_element.range.start.character,
                            },
                            end: lsp_types::Position {
                                line: input_value_element.range.end.line,
                                character: input_value_element.range.end.character,
                            },
                        },
                        format!(
                            "Invalid input value. Value needs to be one of: '{}'.",
                            options.join(", ")
                        ),
                    ));
                }
            }

            // check if it matches to the spec pattern
            if let Some(pattern) = &spec_definition.regex {
                if let Ok(regex) = Regex::new(pattern.trim_matches('/')) {
                    if !regex.is_match(input_value) {
                        diagnostics.push(Diagnostic::new_simple(
                            lsp_types::Range {
                                start: lsp_types::Position {
                                    line: input_value_element.range.start.line,
                                    character: input_value_element.range.start.character,
                                },
                                end: lsp_types::Position {
                                    line: input_value_element.range.end.line,
                                    character: input_value_element.range.end.character,
                                },
                            },
                            format!("Invalid value. Value needs to match the pattern: {pattern}"),
                        ));
                    }
                } else {
                    error!("could not parse regex from input spec regex: {pattern}");
                }
            }
        }
    } else {
        diagnostics.push(Diagnostic::new_simple(
            lsp_types::Range {
                start: lsp_types::Position {
                    line: i.range.start.line,
                    character: i.range.start.character,
                },
                end: lsp_types::Position {
                    line: i.range.end.line,
                    character: i.range.end.character,
                },
            },
            "Missing value.".to_string(),
        ));
    }
}
#[cfg(test)]
mod tests {
    use super::LSPHandlers;
    use lsp_types::Url;
    use std::fs::{self, File};
    use std::path::PathBuf;
    use tempfile::{tempdir, TempDir};

    // Helper function to create a temporary directory and files within it
    fn setup_test_environment(files_to_create: &[&str], dirs_to_create: &[&str]) -> TempDir {
        let dir = tempdir().expect("Failed to create temporary directory");
        let root_path = dir.path();

        for file in files_to_create {
            let file_path = root_path.join(file);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).expect("Failed to create parent directories");
            }
            File::create(&file_path).expect("Failed to create file");
        }

        for dir_path in dirs_to_create {
            fs::create_dir_all(root_path.join(dir_path)).expect("Failed to create directory");
        }

        dir
    }

    #[test]
    fn test_resolve_root_files_empty_input() {
        let tmp_dir = setup_test_environment(&[], &[]);
        let project_root_url = Url::from_directory_path(tmp_dir.path()).unwrap();
        let files: Vec<String> = vec![];

        let resolved = LSPHandlers::resolve_root_files(files, &project_root_url);
        assert!(resolved.is_empty());
    }

    #[test]
    fn test_resolve_root_files_single_file() {
        let tmp_dir = setup_test_environment(&["test_file.gitlab-ci.yml"], &[]);
        let project_root_url = Url::from_directory_path(tmp_dir.path()).unwrap();
        let files = vec!["test_file.gitlab-ci.yml".to_string()];

        let resolved = LSPHandlers::resolve_root_files(files, &project_root_url);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], tmp_dir.path().join("test_file.gitlab-ci.yml"));
    }

    #[test]
    fn test_resolve_root_files_multiple_files() {
        let tmp_dir = setup_test_environment(
            &[
                "file1.gitlab-ci.yml",
                "subdir/file2.gitlab-ci.yml",
                "another/path/file3.gitlab-ci.yml",
            ],
            &[],
        );
        let project_root_url = Url::from_directory_path(tmp_dir.path()).unwrap();
        let files = vec![
            "file1.gitlab-ci.yml".to_string(),
            "subdir/file2.gitlab-ci.yml".to_string(),
            "another/path/file3.gitlab-ci.yml".to_string(),
        ];

        let mut resolved = LSPHandlers::resolve_root_files(files, &project_root_url);
        resolved.sort(); // Sort to ensure consistent order for assertion

        let mut expected: Vec<PathBuf> = vec![
            tmp_dir.path().join("file1.gitlab-ci.yml"),
            tmp_dir.path().join("subdir/file2.gitlab-ci.yml"),
            tmp_dir.path().join("another/path/file3.gitlab-ci.yml"),
        ];
        expected.sort();

        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved, expected);
    }

    #[test]
    fn test_resolve_root_files_non_existent_file() {
        let tmp_dir = setup_test_environment(&[], &[]);
        let project_root_url = Url::from_directory_path(tmp_dir.path()).unwrap();
        let files = vec!["non_existent_file.gitlab-ci.yml".to_string()];

        let resolved = LSPHandlers::resolve_root_files(files, &project_root_url);
        assert!(resolved.is_empty()); // Non-existent files should not be resolved
    }

    #[test]
    fn test_resolve_root_files_directory() {
        let tmp_dir = setup_test_environment(
            &[
                "dir1/test1.gitlab-ci.yml",
                "dir1/subdir/test2.gitlab-ci.yml",
                "dir2/test3.gitlab-ci.yml",
            ],
            &["dir1", "dir1/subdir", "dir2"],
        );
        let project_root_url = Url::from_directory_path(tmp_dir.path()).unwrap();
        let files = vec!["dir1".to_string(), "dir2".to_string()];

        let mut resolved = LSPHandlers::resolve_root_files(files, &project_root_url);
        resolved.sort();

        let mut expected: Vec<PathBuf> = vec![
            tmp_dir.path().join("dir1/test1.gitlab-ci.yml"),
            tmp_dir.path().join("dir1/subdir/test2.gitlab-ci.yml"),
            tmp_dir.path().join("dir2/test3.gitlab-ci.yml"),
        ];
        expected.sort();

        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved, expected);
    }

    #[test]
    fn test_resolve_root_files_glob_pattern() {
        let tmp_dir = setup_test_environment(
            &[
                "file_a.gitlab-ci.yml",
                "file_b.gitlab-ci.yml",
                "other_file.txt",
            ],
            &[],
        );
        let project_root_url = Url::from_directory_path(tmp_dir.path()).unwrap();
        let files = vec!["file_*.gitlab-ci.yml".to_string()];

        let mut resolved = LSPHandlers::resolve_root_files(files, &project_root_url);
        resolved.sort();

        let mut expected: Vec<PathBuf> = vec![
            tmp_dir.path().join("file_a.gitlab-ci.yml"),
            tmp_dir.path().join("file_b.gitlab-ci.yml"),
        ];
        expected.sort();

        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved, expected);
    }

    #[test]
    fn test_resolve_root_files_mixed_input() {
        let tmp_dir = setup_test_environment(
            &[
                "file1.gitlab-ci.yml",
                "dir_mix/file2.gitlab-ci.yml",
                "dir_mix/subdir_mix/file3.gitlab-ci.yml",
                "glob_a.gitlab-ci.yml",
                "glob_b.gitlab-ci.yml",
                "another_file.txt",
            ],
            &["dir_mix", "dir_mix/subdir_mix"],
        );
        let project_root_url = Url::from_directory_path(tmp_dir.path()).unwrap();
        let files = vec![
            "file1.gitlab-ci.yml".to_string(),
            "dir_mix".to_string(),
            "glob_*.gitlab-ci.yml".to_string(),
            "non_existent.gitlab-ci.yml".to_string(), // Should be ignored
        ];

        let mut resolved = LSPHandlers::resolve_root_files(files, &project_root_url);
        resolved.sort();

        let mut expected: Vec<PathBuf> = vec![
            tmp_dir.path().join("file1.gitlab-ci.yml"),
            tmp_dir.path().join("dir_mix/file2.gitlab-ci.yml"),
            tmp_dir
                .path()
                .join("dir_mix/subdir_mix/file3.gitlab-ci.yml"),
            tmp_dir.path().join("glob_a.gitlab-ci.yml"),
            tmp_dir.path().join("glob_b.gitlab-ci.yml"),
        ];
        expected.sort();

        assert_eq!(resolved.len(), 5);
        assert_eq!(resolved, expected);
    }
}
