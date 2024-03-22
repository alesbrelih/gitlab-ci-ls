use gitlab_parser::events::LspEvents;
use gitlab_parser::LSPResult;
use log::{debug, error, info, warn, LevelFilter};
use serde::{Deserialize, Serialize};

use lsp_server::{Connection, Message, Response};
use lsp_types::{
    Hover, HoverContents, LocationLink, MarkedString, Position, ServerCapabilities,
    TextDocumentSyncKind, Url,
};

use std::collections::HashMap;
use std::error::Error;
use std::process::exit;

#[derive(Serialize, Deserialize, Debug)]
struct InitializationOptions {
    #[serde(default = "default_package_map")]
    package_map: HashMap<String, String>,

    #[serde(default = "default_log_path")]
    log_path: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct InitializationParams {
    #[serde(rename = "initializationOptions")]
    initialization_options: InitializationOptions,

    #[serde(rename = "rootPath")]
    root_path: String,
}

fn default_package_map() -> HashMap<String, String> {
    HashMap::new()
}

fn default_log_path() -> String {
    "/dev/null".to_string()
}

fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();

    let server_capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(lsp_types::TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::FULL,
        )),
        hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
        definition_provider: Some(lsp_types::OneOf::Left(true)),

        ..Default::default()
    })?;

    let initialization_params = connection.initialize(server_capabilities)?;

    warn!("params {:?}", initialization_params);

    let init_params = match serde_json::from_value::<InitializationParams>(initialization_params) {
        Ok(p) => p,
        Err(err) => {
            error!("error deserializing init params; got err {}", err);

            InitializationParams {
                root_path: String::new(),
                initialization_options: InitializationOptions {
                    log_path: String::from("/dev/null"),
                    package_map: HashMap::new(),
                },
            }
        }
    };

    info!("init_params {:?}", init_params);
    simple_logging::log_to_file(
        init_params.initialization_options.log_path,
        LevelFilter::Warn,
    )?;

    let lsp_events = LspEvents::new(gitlab_parser::LSPConfig {
        cache_path: format!("{}/.gitlab-ls/cache/", std::env::var("HOME")?),
        package_map: init_params.initialization_options.package_map,
        root_dir: init_params.root_path,
    });

    debug!("initialized");

    for msg in &connection.receiver {
        info!("receiver message {:?}", msg);

        let result = match msg {
            // TODO: implement workspace/didChangeConfiguration
            Message::Notification(notification) => match notification.method.as_str() {
                "textDocument/didOpen" => lsp_events.on_open(notification),
                "textDocument/didChange" => lsp_events.on_change(notification),
                _ => {
                    warn!("invalid notification method: {:?}", notification);
                    None
                }
            },
            Message::Request(request) => match request.method.as_str() {
                "textDocument/hover" => lsp_events.on_hover(request),
                "textDocument/definition" => lsp_events.on_definition(request),
                "shutdown" => exit(0),
                method => {
                    warn!("invalid request method: {:?}", method);
                    None
                }
            },
            m => {
                warn!("unhandled message {:?}", m);
                None
            }
        };

        info!("got result {:?}", &result);

        let sent = match result {
            Some(LSPResult::Hover(hover_result)) => {
                info!("send hover msg: {:?}", hover_result);

                let msg = Message::Response(Response {
                    id: hover_result.id,
                    result: serde_json::to_value(Hover {
                        contents: HoverContents::Scalar(MarkedString::String(hover_result.content)),
                        range: None,
                    })
                    .ok(),
                    error: None,
                });

                connection.sender.send(msg)
            }
            Some(LSPResult::Definition(definition_result)) => {
                info!("send definition msg: {:?}", definition_result);

                let locations: Vec<LocationLink> = definition_result
                    .locations
                    .iter()
                    .map(|l| LocationLink {
                        target_uri: Url::parse(&l.uri).unwrap(),
                        origin_selection_range: None,
                        target_selection_range: lsp_types::Range {
                            start: Position {
                                character: l.range.start.character,
                                line: l.range.start.line,
                            },
                            end: Position {
                                character: l.range.end.character,
                                line: l.range.end.line,
                            },
                        },
                        target_range: lsp_types::Range {
                            start: Position {
                                character: l.range.start.character,
                                line: l.range.start.line,
                            },
                            end: Position {
                                character: l.range.end.character,
                                line: l.range.end.line,
                            },
                        },
                    })
                    .collect();

                let msg = Message::Response(Response {
                    id: definition_result.id,
                    result: serde_json::to_value(locations).ok(),
                    error: None,
                });

                connection.sender.send(msg)
            }
            None => continue,
        };

        match sent {
            Err(err) => {
                error!("error sending: {:?}", err);
            }
            Ok(_) => continue,
        }
    }

    io_threads.join()?;

    Ok(())
}
