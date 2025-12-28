use std::process::exit;
use std::sync::Arc;

use log::{error, info, warn};
use lsp_server::{Connection, Message, Response, ResponseError};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionTextEdit, Hover, HoverContents,
    LocationLink, MarkedString, MarkupContent, Position, TextEdit, WorkspaceEdit,
};
use reqwest::Url;

use crate::gitlab_ci_ls_parser::LSPResult;

use super::{
    handlers::LSPHandlers, CompletionResult, DefinitionResult, DiagnosticsNotification,
    HoverResult, PrepareRenameResult, ReferencesResult, RenameResult,
};

pub struct Messages {
    connection: Connection,
    events: Arc<LSPHandlers>,
}

impl Messages {
    pub fn new(connection: Connection, events: Arc<LSPHandlers>) -> Self {
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
                    warn!("invalid notification method: {notification:?}");
                    None
                }
            },
            Message::Request(request) => match request.method.as_str() {
                "textDocument/hover" => self.events.on_hover(request),
                "textDocument/definition" => self.events.on_definition(request),
                "textDocument/references" => self.events.on_references(request),
                "textDocument/completion" => self.events.on_completion(request),
                "textDocument/prepareRename" => self.events.on_prepare_rename(request),
                "textDocument/rename" => self.events.on_rename(request),
                "shutdown" => {
                    error!("SHUTDOWN!!");
                    exit(0);
                }
                method => {
                    warn!("invalid request method: {method:?}");
                    None
                }
            },
            Message::Response(request) => {
                warn!("unhandled message {request:?}");
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
            info!("send hover msg: {hover_result:?}");
            Some(hover(hover_result))
        }
        Some(LSPResult::Completion(completion_result)) => {
            info!("send completion msg: {completion_result:?}");
            Some(completion(completion_result))
        }
        Some(LSPResult::Definition(definition_result)) => {
            info!("send definition msg: {definition_result:?}");
            Some(definition(definition_result))
        }
        Some(LSPResult::References(references_result)) => {
            info!("send references msg: {references_result:?}");
            Some(references(references_result))
        }
        Some(LSPResult::Diagnostics(diagnostics_result)) => {
            info!("send definition msg: {diagnostics_result:?}");
            Some(diagnostics(diagnostics_result))
        }
        Some(LSPResult::PrepareRename(res)) => {
            info!("send prepare rename msg: {res:?}");
            Some(prepare_rename(res))
        }
        Some(LSPResult::Rename(res)) => {
            info!("send prepare rename msg: {res:?}");
            Some(rename(res))
        }
        Some(LSPResult::Error(err)) => {
            error!("error handling message: {msg:?} got error: {err:?}");
            null_response(msg)
        }
        None => null_response(msg),
    }
}

fn rename(res: RenameResult) -> Message {
    let mut res = Response {
        id: res.id,
        result: serde_json::to_value(WorkspaceEdit {
            changes: res.edits,
            ..Default::default()
        })
        .ok(),
        error: None,
    };

    if let Some(err) = res.error {
        res.error = Some(ResponseError {
            code: -1,
            message: err.message,
            data: None,
        });
    }

    Message::Response(res)
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

fn diagnostics(notification: DiagnosticsNotification) -> Message {
    Message::Notification(lsp_server::Notification {
        method: "textDocument/publishDiagnostics".to_string(),
        params: serde_json::to_value(lsp_types::PublishDiagnosticsParams {
            uri: notification.uri,
            diagnostics: notification.diagnostics,
            version: None,
        })
        .unwrap(),
    })
}

fn prepare_rename(res: PrepareRenameResult) -> Message {
    let mut r = Response {
        id: res.id,
        result: None,
        error: None,
    };

    if let Some(range) = res.range {
        r.result =
            serde_json::to_value(lsp_types::PrepareRenameResponse::Range(lsp_types::Range {
                start: Position {
                    line: range.start.line,
                    character: range.start.character,
                },
                end: Position {
                    line: range.end.line,
                    character: range.end.character,
                },
            }))
            .ok();
    }

    if let Some(err) = res.err {
        r.error = Some(ResponseError {
            code: -1,
            message: err,
            data: None,
        });
    }

    Message::Response(r)
}
