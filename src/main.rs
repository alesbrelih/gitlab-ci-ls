use anyhow::anyhow;
use clap::Parser;
use gitlab_ci_ls_parser::LSPExperimental;
use log::{error, info, warn, LevelFilter};
use regex::Regex;
use serde::{Deserialize, Serialize};

use lsp_server::Connection;
use lsp_types::{ServerCapabilities, TextDocumentSyncKind, WorkDoneProgressOptions};

use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::process::Command;

use crate::gitlab_ci_ls_parser::fs_utils::{FSUtils, FSUtilsImpl};
use crate::gitlab_ci_ls_parser::messages;

mod gitlab_ci_ls_parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {}

#[derive(Serialize, Deserialize, Debug)]
struct InitializationOptions {
    #[serde(default = "default_package_map")]
    package_map: HashMap<String, String>,

    #[serde(default = "default_log_path")]
    log_path: String,

    #[serde(rename = "cache", default = "default_cache_path")]
    cache_path: String,

    #[serde(default = "default_options")]
    options: Options,
}

#[derive(Serialize, Deserialize, Debug)]
struct Options {
    #[serde(default = "default_dependencies_autocomplete_stage_filtering")]
    dependencies_autocomplete_stage_filtering: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct InitializationParams {
    #[serde(rename = "initializationOptions")]
    initialization_options: InitializationOptions,

    #[serde(rename = "rootPath")]
    root_path: String,
}

fn default_options() -> Options {
    Options {
        dependencies_autocomplete_stage_filtering: false,
    }
}

fn default_dependencies_autocomplete_stage_filtering() -> bool {
    false
}

fn default_package_map() -> HashMap<String, String> {
    HashMap::new()
}

fn default_log_path() -> String {
    "/dev/null".to_string()
}

fn default_cache_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();

    format!("{home}/.cache/.gitlab-ci-ls")
}

#[allow(clippy::too_many_lines)]
fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    Args::parse();

    let (connection, io_threads) = Connection::stdio();

    let server_capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(lsp_types::TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::FULL,
        )),
        hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
        definition_provider: Some(lsp_types::OneOf::Left(true)),
        references_provider: Some(lsp_types::OneOf::Left(true)),
        completion_provider: Some(lsp_types::CompletionOptions {
            resolve_provider: Some(false),
            trigger_characters: Some(vec![
                ".".to_string(),
                ":".to_string(),
                " ".to_string(),
                "$".to_string(),
            ]),
            work_done_progress_options: WorkDoneProgressOptions {
                work_done_progress: None,
            },
            all_commit_characters: None,
            completion_item: None,
        }),
        rename_provider: Some(lsp_types::OneOf::Right(lsp_types::RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: WorkDoneProgressOptions {
                ..Default::default()
            },
        })),
        ..Default::default()
    })?;

    let initialization_params = connection.initialize(server_capabilities)?;
    let init_params =
        match serde_json::from_value::<InitializationParams>(initialization_params.clone()) {
            Ok(p) => p,
            Err(err) => {
                error!("error deserializing init params; got err {}", err);

                InitializationParams {
                    root_path: String::new(),
                    initialization_options: InitializationOptions {
                        log_path: default_log_path(),
                        package_map: HashMap::new(),
                        cache_path: default_cache_path(),
                        options: Options {
                            dependencies_autocomplete_stage_filtering:
                                default_dependencies_autocomplete_stage_filtering(),
                        },
                    },
                }
            }
        };

    let home_path = std::env::var("HOME")?;
    let fs_utils = FSUtilsImpl::new(home_path);

    simple_logging::log_to_file(
        fs_utils.create_log_file(&init_params.initialization_options.log_path),
        LevelFilter::Warn,
    )?;

    let remote_urls = match get_git_remotes(&init_params.root_path) {
        Ok(u) => u,
        Err(err) => {
            error!(
                "error getting git remotes at: {}; got err: {:?}",
                &init_params.root_path, err
            );
            vec![]
        }
    };

    if let Err(err) = save_base_files(&init_params, &fs_utils) {
        error!("error saving base files; got err: {err}");
    }

    let lsp_events = gitlab_ci_ls_parser::handlers::LSPHandlers::new(
        gitlab_ci_ls_parser::LSPConfig {
            cache_path: fs_utils
                .get_path(&init_params.initialization_options.cache_path)
                .to_string_lossy()
                .to_string(),
            package_map: init_params.initialization_options.package_map,
            remote_urls,
            root_dir: init_params.root_path,
            experimental: LSPExperimental {
                dependencies_autocomplete_stage_filtering: init_params
                    .initialization_options
                    .options
                    .dependencies_autocomplete_stage_filtering,
            },
        },
        Box::new(fs_utils),
    );

    info!("initialized");

    messages::Messages::new(connection, lsp_events).handle();

    io_threads.join()?;

    Ok(())
}

fn get_git_remotes(root_path: &str) -> anyhow::Result<Vec<String>> {
    let output = Command::new("git")
        .args(["-C", root_path, "remote", "-v"])
        .output()?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "error listing remotes: {:?}",
            std::str::from_utf8(&output.stderr)
        ));
    }

    let mut remotes = std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter_map(get_remote_hosts)
        .collect::<Vec<String>>();

    remotes.dedup();

    Ok(remotes)
}

fn save_base_files(
    init_params: &InitializationParams,
    fs_utils: &FSUtilsImpl,
) -> anyhow::Result<()> {
    let base_path = format!(
        "{}base",
        fs_utils
            .get_path(&init_params.initialization_options.cache_path)
            .to_string_lossy()
    );
    let _ = fs_utils.create_dir_all(&base_path);

    let gitlab_predefined = include_str!("./resources/gitlab_predefined_vars.yaml");
    let gitlab_predefined_path = format!("{base_path}/gitlab_predefined_vars.yaml");
    info!("predefined path: {}", gitlab_predefined_path);

    let mut file = File::create(&gitlab_predefined_path)
        .map_err(|e| anyhow!("error creating file: {gitlab_predefined_path}; got err: {e}"))?;
    file.write_all(gitlab_predefined.as_bytes())?;

    Ok(())
}

fn get_remote_hosts(remote: &str) -> Option<String> {
    let re = Regex::new(r"^(ssh://)?([^:\s/]+@[^:/]+(?::\d+)?[:/])|(https://[^:/]+[:/])")
        .expect("Invalid REGEX");

    let captures = re.captures(remote)?;

    Some(captures[0].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_remote_urls_full_scheme() {
        assert_eq!(
            get_remote_hosts("ssh://git@something.host.online:4242/myrepo/wow.git"),
            Some("ssh://git@something.host.online:4242/".to_string())
        );
    }

    #[test]
    fn test_get_remote_urls_basic() {
        assert_eq!(
            get_remote_hosts("git@something.host.online:myrepo/wow.git"),
            Some("git@something.host.online:".to_string())
        );
    }

    #[test]
    fn test_get_remote_urls_https() {
        assert_eq!(
            get_remote_hosts("https://gitlab.com/group/othergroup/project.git"),
            Some("https://gitlab.com/".to_string())
        );
    }

    #[test]
    fn test_get_remote_urls_https_custom_instance() {
        assert_eq!(
            get_remote_hosts("https://gitlab.instance.com/group/othergroup/project.git"),
            Some("https://gitlab.instance.com/".to_string())
        );
    }
}
