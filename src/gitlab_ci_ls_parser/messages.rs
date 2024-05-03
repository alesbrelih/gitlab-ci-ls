use std::process::exit;

use log::{error, info, warn};
use lsp_server::{Connection, Message, Response};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionTextEdit,
    FullDocumentDiagnosticReport, Hover, HoverContents, LocationLink, MarkedString, MarkupContent,
    Position, TextEdit,
};
use reqwest::Url;

use crate::gitlab_ci_ls_parser::LSPResult;

use super::{
    handlers::LSPHandlers, CompletionResult, DefinitionResult, DiagnosticsResult, HoverResult,
    ReferencesResult,
};

pub struct Messages {
    connection: Connection,
    events: LSPHandlers,
}

impl Messages {
    pub fn new(connection: Connection, events: LSPHandlers) -> Self {
        Self { connection, events }
    }

    pub fn handle(&self) {
        self.connection
            .receiver
            .iter()
            .for_each(|msg| self.handle_message(&msg));
    }

    fn handle_message(&self, msg: &Message) {
        info!("received message {msg:?}");

        let msg_clone = msg.clone();
        let result = match msg_clone {
            // TODO: implement workspace/didChangeConfiguration
            Message::Notification(notification) => match notification.method.as_str() {
                "textDocument/didOpen" => self.events.on_open(notification),
                "textDocument/didChange" => self.events.on_change(notification),
                "textDocument/didSave" => self.events.on_save(notification),
                _ => {
                    warn!("invalid notification method: {:?}", notification);
                    None
                }
            },
            Message::Request(request) => match request.method.as_str() {
                "textDocument/hover" => self.events.on_hover(request),
                "textDocument/definition" => self.events.on_definition(request),
                "textDocument/references" => self.events.on_references(request),
                "textDocument/completion" => self.events.on_completion(request),
                "textDocument/diagnostic" => self.events.on_diagnostic(request),
                "shutdown" => {
                    error!("SHUTDOWN!!");
                    exit(0);
                }
                method => {
                    warn!("invalid request method: {:?}", method);
                    None
                }
            },
            Message::Response(request) => {
                warn!("unhandled message {:?}", request);
                None
            }
        };

        let sent = match handle_result(msg, result) {
            Some(msg) => self.connection.sender.send(msg),
            None => Ok(()),
        };

        if let Err(err) = sent {
            error!("error handling message: {err}");
        }
    }
}

fn handle_result(msg: &Message, result: Option<LSPResult>) -> Option<Message> {
    info!("got result {:?}", &result);

    match result {
        Some(LSPResult::Hover(hover_result)) => {
            info!("send hover msg: {:?}", hover_result);
            Some(hover(hover_result))
        }
        Some(LSPResult::Completion(completion_result)) => {
            info!("send completion msg: {:?}", completion_result);
            Some(completion(completion_result))
        }
        Some(LSPResult::Definition(definition_result)) => {
            info!("send definition msg: {:?}", definition_result);
            Some(definition(definition_result))
        }
        Some(LSPResult::References(references_result)) => {
            info!("send references msg: {:?}", references_result);
            Some(references(references_result))
        }
        Some(LSPResult::Diagnostics(diagnostics_result)) => {
            info!("send definition msg: {:?}", diagnostics_result);
            Some(diagnostics(diagnostics_result))
        }
        Some(LSPResult::Error(err)) => {
            error!("error handling message: {:?} got error: {:?}", msg, err);
            null_response(msg)
        }
        None => null_response(msg),
    }
}

fn null_response(msg: &Message) -> Option<Message> {
    match msg {
        Message::Request(req) => Some(Message::Response(Response {
            id: req.clone().id,
            result: Some(serde_json::Value::Null),
            error: None,
        })),
        _ => None,
    }
}

fn hover(result: HoverResult) -> Message {
    Message::Response(Response {
        id: result.id,
        result: serde_json::to_value(Hover {
            contents: HoverContents::Scalar(MarkedString::String(result.content)),
            range: None,
        })
        .ok(),
        error: None,
    })
}

fn completion(result: CompletionResult) -> Message {
    Message::Response(Response {
        id: result.id,
        result: serde_json::to_value(CompletionList {
            items: result
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

                    if let Some(documentation) = c.details.clone() {
                        item.documentation =
                            Some(lsp_types::Documentation::MarkupContent(MarkupContent {
                                kind: lsp_types::MarkupKind::Markdown,
                                value: documentation.clone(),
                            }));
                    }

                    item
                })
                .collect(),
            is_incomplete: false,
        })
        .ok(),
        error: None,
    })
}

fn definition(result: DefinitionResult) -> Message {
    let locations: Vec<LocationLink> = result
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

    Message::Response(Response {
        id: result.id,
        result: serde_json::to_value(locations).ok(),
        error: None,
    })
}

fn references(result: ReferencesResult) -> Message {
    let locations: Vec<LocationLink> = result
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

    Message::Response(Response {
        id: result.id,
        result: serde_json::to_value(locations).ok(),
        error: None,
    })
}

fn diagnostics(result: DiagnosticsResult) -> Message {
    Message::Response(Response {
        id: result.id,
        result: serde_json::to_value(FullDocumentDiagnosticReport {
            items: result.diagnostics,
            ..Default::default()
        })
        .ok(),
        error: None,
    })
}
