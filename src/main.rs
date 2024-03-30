use git2::Repository;
use gitlab_parser::handlers::LSPHandlers;
use gitlab_parser::LSPResult;
use log::{error, info, warn, LevelFilter};
use regex::Regex;
use serde::{Deserialize, Serialize};

use lsp_server::{Connection, Message, Response};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionTextEdit,
    DiagnosticServerCapabilities, DocumentFilter, FullDocumentDiagnosticReport, Hover,
    HoverContents, LocationLink, MarkedString, MarkupContent, Position, ServerCapabilities,
    TextDocumentSyncKind, TextEdit, Url, WorkDoneProgressOptions,
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
        completion_provider: Some(lsp_types::CompletionOptions {
            resolve_provider: Some(false),
            trigger_characters: Some(vec![".".to_string(), " ".to_string(), "$".to_string()]),
            work_done_progress_options: WorkDoneProgressOptions {
                work_done_progress: None,
            },
            all_commit_characters: None,
            completion_item: None,
        }),
        diagnostic_provider: Some(DiagnosticServerCapabilities::RegistrationOptions(
            lsp_types::DiagnosticRegistrationOptions {
                diagnostic_options: lsp_types::DiagnosticOptions {
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: None,
                    },
                    identifier: None,
                    workspace_diagnostics: false,
                    inter_file_dependencies: true,
                },
                static_registration_options: lsp_types::StaticRegistrationOptions { id: None },
                text_document_registration_options: lsp_types::TextDocumentRegistrationOptions {
                    document_selector: Some(vec![DocumentFilter {
                        pattern: Some(String::from("*gitlab-ci*")),
                        scheme: Some("file".into()),
                        language: Some("yaml".into()),
                    }]),
                },
            },
        )),

        ..Default::default()
    })?;

    let initialization_params = connection.initialize(server_capabilities)?;

    error!("params {:?}", initialization_params);

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

    simple_logging::log_to_file(
        &init_params.initialization_options.log_path,
        LevelFilter::Warn,
    )?;

    let repo = Repository::open(&init_params.root_path)?;
    let remote_urls: Vec<String> = repo
        .remotes()?
        .iter()
        .flatten()
        .flat_map(|r_name| repo.find_remote(r_name))
        .filter_map(|remote| remote.url().map(|u| u.to_string()))
        .filter_map(get_remote_hosts)
        .collect();

    // get_remote_urls(repo.remotes()?.iter())?;

    let lsp_events = LSPHandlers::new(gitlab_parser::LSPConfig {
        cache_path: format!("{}/.gitlab-ls/cache/", std::env::var("HOME")?),
        package_map: init_params.initialization_options.package_map,
        remote_urls,
        root_dir: init_params.root_path,
    });

    info!("initialized");

    for msg in &connection.receiver {
        info!("received message {:?}", msg);

        let msg_clone = msg.clone();
        let result = match msg_clone {
            // TODO: implement workspace/didChangeConfiguration
            Message::Notification(notification) => match notification.method.as_str() {
                "textDocument/didOpen" => lsp_events.on_open(notification),
                "textDocument/didChange" => lsp_events.on_change(notification),
                "textDocument/didSave" => lsp_events.on_save(notification),
                _ => {
                    warn!("invalid notification method: {:?}", notification);
                    None
                }
            },
            Message::Request(request) => match request.method.as_str() {
                "textDocument/hover" => lsp_events.on_hover(request),
                "textDocument/definition" => lsp_events.on_definition(request),
                "textDocument/completion" => lsp_events.on_completion(request),
                "textDocument/diagnostic" => lsp_events.on_diagnostic(request),
                "shutdown" => {
                    error!("SHUTDOWN!!");
                    exit(0);
                }
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
            Some(LSPResult::Completion(completion_result)) => {
                info!("send completion msg: {:?}", completion_result);

                let msg = Message::Response(Response {
                    id: completion_result.id,
                    result: serde_json::to_value(CompletionList {
                        items: completion_result
                            .list
                            .iter()
                            .map(|c| {
                                let mut item = CompletionItem {
                                    label: c.label.clone(),
                                    kind: Some(CompletionItemKind::KEYWORD),
                                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                                        new_text: c.label.clone(),
                                        range: lsp_types::Range {
                                            start: Position {
                                                line: c.location.range.start.line,
                                                character: c.location.range.start.character,
                                            },
                                            end: Position {
                                                line: c.location.range.end.line,
                                                character: c.location.range.end.character,
                                            },
                                        },
                                    })),
                                    ..Default::default()
                                };

                                error!("compeltionItem: {:?}", &item.text_edit);

                                if let Some(documentation) = c.details.clone() {
                                    item.documentation = Some(
                                        lsp_types::Documentation::MarkupContent(MarkupContent {
                                            kind: lsp_types::MarkupKind::Markdown,
                                            value: format!(
                                                "```yaml\r\n{}\r\n```",
                                                documentation.clone()
                                            ),
                                        }),
                                    );
                                }

                                item
                            })
                            .collect(),
                        is_incomplete: false,
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
            Some(LSPResult::Diagnostics(diagnostics)) => {
                let msg = Message::Response(Response {
                    id: diagnostics.id,
                    result: serde_json::to_value(FullDocumentDiagnosticReport {
                        items: diagnostics.diagnostics,
                        ..Default::default()
                    })
                    .ok(),
                    error: None,
                });

                connection.sender.send(msg)
            }
            None => match msg {
                Message::Request(req) => {
                    let msg = Message::Response(Response {
                        id: req.id,
                        result: Some(serde_json::Value::Null),
                        error: None,
                    });

                    connection.sender.send(msg)
                }
                _ => Ok(()),
            },
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

fn get_remote_hosts(remote: String) -> Option<String> {
    let re = Regex::new(r"^(ssh://)?([^:/]+@[^:/]+(?::\d+)?[:/])").expect("Invalid REGEX");
    let captures = re.captures(remote.as_str())?;

    Some(captures[0].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_remote_urls_full_scheme() {
        assert_eq!(
            get_remote_hosts("ssh://git@something.host.online:4242/myrepo/wow.git".to_string()),
            Some("ssh://git@something.host.online:4242/".to_string())
        );
    }

    #[test]
    fn test_get_remote_urls_basic() {
        assert_eq!(
            get_remote_hosts("git@something.host.online:myrepo/wow.git".to_string()),
            Some("git@something.host.online:".to_string())
        );
    }
}
