use std::{collections::HashMap, path::PathBuf, sync::Mutex};

use log::{debug, error, info};
use lsp_server::{Notification, Request};
use lsp_types::{
    request::GotoTypeDefinitionParams, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
    HoverParams, Url,
};
use yaml_rust::{yaml::Hash, Yaml, YamlEmitter, YamlLoader};

use crate::{
    parser::{self, ParserUtils},
    DefinitionResult, HoverResult, LSPConfig, LSPLocation, LSPResult,
};

pub struct LspEvents {
    cfg: LSPConfig,
    store: Mutex<HashMap<String, String>>,
    parser: parser::Parser,
}

impl LspEvents {
    pub fn new(cfg: LSPConfig) -> LspEvents {
        let store = Mutex::new(HashMap::new());

        let events = LspEvents {
            cfg: cfg.clone(),
            store,
            parser: parser::Parser::new(cfg.package_map, cfg.cache_path),
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
        let uri = &params.text_document_position_params.text_document.uri;
        let document = store.get::<String>(&uri.clone().into())?;

        let position = params.text_document_position_params.position;
        let line = document.lines().nth(position.line as usize)?;

        let word =
            ParserUtils::extract_word(line, position.character as usize)?.trim_end_matches(':');

        let mut hover = String::new();

        for (content_uri, content) in store.iter() {
            let (node_key, node_value) = match ParserUtils::get_root_node(content, word) {
                Some((node_key, node_value)) => (node_key, node_value),
                _ => continue,
            };

            // Check if we found the same line that triggered the hover event and discard it
            // adding format : because yaml parser removes it from the key
            if content_uri.ends_with(uri.as_str()) && line.eq(&format!("{}:", node_key.as_str()?)) {
                continue;
            }

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

        let files =
            self.parser
                .parse_contents(&params.text_document.uri, &params.text_document.text, true);

        if let Some(files) = files {
            for file in files {
                store.insert(file.path, file.content);
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

        let word =
            ParserUtils::extract_word(line, position.character as usize)?.trim_end_matches(':');
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

            for (key, _) in yaml_content.as_hash()? {
                if key.as_str()? == word {
                    // shouldn't push when we use go to definition on a root node and this
                    // is the same node
                    // But we need to allow it because gitlab is based on inheritence
                    if document_uri.as_str().ends_with(uri)
                        && line.eq(&format!("{}:", key.as_str()?))
                    {
                        continue;
                    }

                    locations.push(LSPLocation {
                        uri: uri.clone(),
                        range: ParserUtils::find_position(content, word)?,
                    });
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
        let files = self.parser.parse_contents(&uri, &root_file_content, true);
        if let Some(files) = files {
            for file in files {
                info!("found file: {:?}", &file);
                store.insert(file.path, file.content);
            }
        }

        Ok(())
    }
}
