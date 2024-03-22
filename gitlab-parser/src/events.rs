use std::{collections::HashMap, path::PathBuf, sync::Mutex};

use log::{debug, error, info};
use lsp_server::{Notification, Request};
use lsp_types::{
    request::GotoTypeDefinitionParams, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
    HoverParams, Url,
};
use yaml_rust::{yaml::Hash, Yaml, YamlEmitter, YamlLoader};

use crate::{
    utils::{self, get_root_node, parse_contents},
    DefinitionResult, HoverResult, LSPConfig, LSPLocation, LSPResult,
};

pub struct LspEvents {
    cfg: LSPConfig,
    store: Mutex<HashMap<String, String>>,
}

impl LspEvents {
    pub fn new(cfg: LSPConfig) -> LspEvents {
        let events = LspEvents {
            cfg,
            store: Mutex::new(HashMap::new()),
        };

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
        let uri = params.text_document_position_params.text_document.uri;
        let document = store.get::<String>(&uri.clone().into())?;

        let position = params.text_document_position_params.position;
        let line = document.lines().nth(position.line as usize)?;

        let word = utils::extract_word(line, position.character as usize)?.trim_end_matches(':');

        let mut hover = String::new();

        for document in store.values() {
            let (node_key, node_value) = match get_root_node(document, word) {
                Some((node_key, node_value)) => (node_key, node_value),
                _ => continue,
            };

            let mut current_hover = String::new();
            let mut hash = Hash::new();
            hash.insert(node_key, node_value);

            let mut emitter = YamlEmitter::new(&mut current_hover);
            emitter.dump(&Yaml::Hash(hash)).unwrap();

            current_hover = current_hover.trim_start_matches("---\n").to_string();

            if !hover.is_empty() {
                hover = format!("{}\r\n--------\r\n", hover);
            }

            hover = format!("{}{}", hover, current_hover);
        }

        hover = format!("```yaml \r\n{}\r\n```", hover);
        // TODO: support multiple hovers?

        Some(LSPResult::Hover(HoverResult {
            id: request.id,
            content: hover,
        }))
    }

    pub fn on_change(&self, notification: Notification) -> Option<LSPResult> {
        let params =
            serde_json::from_value::<DidChangeTextDocumentParams>(notification.params).ok()?;
        if params.content_changes.len() != 1 {
            return None;
        }

        let mut store = self.store.lock().unwrap();
        store.insert(
            params.text_document.uri.clone().into(),
            params.content_changes.first().unwrap().text.clone(),
        );

        None
    }

    pub fn on_open(&self, notification: Notification) -> Option<LSPResult> {
        let params =
            serde_json::from_value::<DidOpenTextDocumentParams>(notification.params).ok()?;

        debug!("started searching");

        let mut store = self.store.lock().unwrap();

        parse_contents(
            &mut store,
            &self.cfg.package_map,
            self.cfg.cache_path.as_str(),
            &params.text_document.uri,
            &params.text_document.text,
            true,
            0,
        );

        debug!("finished searching");

        None
    }

    pub fn on_definition(&self, request: Request) -> Option<LSPResult> {
        let params = serde_json::from_value::<GotoTypeDefinitionParams>(request.params).ok()?;

        let store = self.store.lock().unwrap();
        let uri = params.text_document_position_params.text_document.uri;
        let document = store.get::<String>(&uri.clone().into())?;

        let position = params.text_document_position_params.position;
        let line = document.lines().nth(position.line as usize)?;

        let split: Vec<&str> = line.trim().split(' ').map(|w| w.trim()).collect();

        // Currently just support the extends keyword and - if its array of extends.
        // Not sure if this conditional is even needed.
        if split.len() > 2 {
            debug!("invalid: {:?}", split);
            return None;
        }

        if split.len() == 2 && split[0] != "extends:" && split[0] != "-" {
            debug!("invalid: {:?}", split);
            return None;
        }

        let word = utils::extract_word(line, position.character as usize)?.trim_end_matches(':');
        debug!("word: {}", word);
        let mut locations: Vec<LSPLocation> = vec![];

        for (uri, content) in store.iter() {
            info!("checking uri {}", uri);
            let documents = match YamlLoader::load_from_str(content) {
                Ok(d) => d,
                Err(err) => {
                    error!("error generating yaml from str, err: {}", err);

                    return None;
                }
            };

            let yaml_content = &documents[0];

            debug!("checking uri: {}", uri);

            if let Yaml::Hash(root) = yaml_content {
                for (key, _) in root {
                    if let Yaml::String(key_str) = key {
                        if key_str.as_str() == word {
                            locations.push(LSPLocation {
                                uri: uri.clone(),
                                range: utils::find_position(content, word)?,
                            });
                        }
                    }
                }
            }
        }

        Some(LSPResult::Definition(DefinitionResult {
            id: request.id,
            locations,
        }))
    }

    fn index_workspace(&self, root_dir: &str) -> anyhow::Result<()> {
        let mut store = self.store.lock().unwrap();

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

        parse_contents(
            &mut store,
            &self.cfg.package_map,
            self.cfg.cache_path.as_str(),
            &uri,
            &root_file_content,
            true,
            0,
        );

        Ok(())
    }
}
