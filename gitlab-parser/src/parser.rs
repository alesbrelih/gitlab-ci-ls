use std::{collections::HashMap, env, fs, path, process::Command};

use log::{debug, error, info};
use lsp_types::{Position, Url};
use yaml_rust::{Yaml, YamlLoader};

use crate::{treesitter::Treesitter, GitlabElement, GitlabFile, GitlabRootNode, ParseResults};

pub struct Parser {
    cache_path: String,
    package_map: HashMap<String, String>,
    remote_urls: Vec<String>,
    treesitter: Treesitter,
}

#[derive(Debug)]
pub struct LocalInclude {
    pub path: String,
}
#[derive(Debug, Default)]
pub struct RemoteInclude {
    pub project: Option<String>,
    pub reference: Option<String>,
    pub file: Option<String>,
}

impl RemoteInclude {
    pub fn is_valid(&self) -> bool {
        self.project.is_some() && self.reference.is_some() && self.file.is_some()
    }
}

#[derive(Debug)]
pub struct IncludeInformation {
    pub remote: Option<RemoteInclude>,
    pub local: Option<LocalInclude>,
}

// TODO: rooot for the case of importing f9
pub enum CompletionType {
    Extend,
    Stage,
    Variable,
    None,
    RootNode,
    Include(IncludeInformation),
}

pub struct ParserUtils {}

impl ParserUtils {
    pub fn strip_quotes(value: &str) -> &str {
        value.trim_matches('\'').trim_matches('"')
    }

    pub fn extract_word(line: &str, char_index: usize) -> Option<&str> {
        if char_index >= line.len() {
            return None;
        }

        let start = line[..char_index]
            .rfind(|c: char| c.is_whitespace())
            .map_or(0, |index| index + 1);

        let end = line[char_index..]
            .find(|c: char| c.is_whitespace())
            .map_or(line.len(), |index| index + char_index);

        Some(&line[start..end])
    }

    pub fn word_before_cursor(
        line: &str,
        char_index: usize,
        predicate: fn(c: char) -> bool,
    ) -> &str {
        if char_index == 0 || char_index > line.len() {
            return "";
        }

        let start = line[..char_index]
            .rfind(predicate)
            .map_or(0, |index| index + 1);

        if start == char_index {
            return "";
        }

        &line[start..char_index]
    }

    pub fn word_after_cursor(line: &str, char_index: usize) -> &str {
        if char_index >= line.len() {
            return "";
        }

        let start = char_index;

        let end = line[start..]
            .char_indices()
            .find(|&(_, c)| c.is_whitespace())
            .map_or(line.len(), |(idx, _)| start + idx);

        &line[start..end]
    }
}

impl Parser {
    pub fn new(
        remote_urls: Vec<String>,
        package_map: HashMap<String, String>,
        cache_path: String,
    ) -> Parser {
        Parser {
            remote_urls,
            package_map,
            cache_path,
            treesitter: Treesitter::new(),
        }
    }

    pub fn get_all_extends(
        &self,
        uri: String,
        content: &str,
        extend_name: Option<&str>,
    ) -> Vec<GitlabElement> {
        self.treesitter.get_all_extends(uri, content, extend_name)
    }

    pub fn get_all_stages(&self, uri: String, content: &str) -> Vec<GitlabElement> {
        self.treesitter.get_all_stages(uri, content)
    }

    pub fn get_position_type(&self, content: &str, position: Position) -> CompletionType {
        self.treesitter.get_position_type(content, position)
    }

    pub fn get_root_node(&self, uri: &str, content: &str, node_key: &str) -> Option<GitlabElement> {
        self.treesitter.get_root_node(uri, content, node_key)
    }

    pub fn parse_contents(&self, uri: &Url, content: &str, _follow: bool) -> Option<ParseResults> {
        let files: Vec<GitlabFile> = vec![];
        let nodes: Vec<GitlabRootNode> = vec![];
        let stages: Vec<GitlabElement> = vec![];
        let variables: Vec<GitlabElement> = vec![];

        let mut parse_results = ParseResults {
            files,
            nodes,
            stages,
            variables,
        };

        self.parse_contents_recursive(&mut parse_results, uri, content, _follow, 0)?;

        Some(parse_results)
    }

    fn parse_contents_recursive(
        &self,
        parse_results: &mut ParseResults,
        uri: &lsp_types::Url,
        content: &str,
        _follow: bool,
        iteration: i32,
    ) -> Option<()> {
        // #safety wow amazed
        if iteration > 10 {
            return None;
        }

        parse_results.files.push(GitlabFile {
            path: uri.as_str().into(),
            content: content.into(),
        });

        parse_results
            .nodes
            .append(&mut self.treesitter.get_all_root_nodes(uri.as_str(), content));

        parse_results
            .variables
            .append(&mut self.treesitter.get_root_variables(uri.as_str(), content));

        // arrays are overriden in gitlab.
        let found_stages = self.treesitter.get_stage_definitions(uri.as_str(), content);
        if !found_stages.is_empty() {
            parse_results.stages = found_stages;
        }

        let element = self
            .treesitter
            .get_root_node(uri.as_str(), content, "include")?;

        let documents = YamlLoader::load_from_str(element.content?.as_str()).ok()?;
        let content = &documents[0];

        if let Yaml::Hash(include_root) = content {
            for (_, root) in include_root {
                if let Yaml::Array(includes) = root {
                    for include in includes {
                        if let Yaml::Hash(item) = include {
                            let mut remote_pkg = String::new();
                            let mut remote_tag = String::new();
                            let mut remote_files: Vec<String> = vec![];

                            for (key, item_value) in item {
                                if _follow && !remote_pkg.is_empty() {
                                    self.fetch_remote_files(
                                        parse_results,
                                        &remote_pkg,
                                        &remote_tag,
                                        &remote_files,
                                    );
                                }

                                if let Yaml::String(key_str) = key {
                                    match key_str.trim().to_lowercase().as_str() {
                                        "local" => {
                                            if let Yaml::String(value) = item_value {
                                                let current_uri = uri.join(value.as_str()).ok()?;
                                                let current_content =
                                                    std::fs::read_to_string(current_uri.path())
                                                        .ok()?;

                                                if _follow {
                                                    self.parse_contents_recursive(
                                                        parse_results,
                                                        &current_uri,
                                                        &current_content,
                                                        _follow,
                                                        iteration + 1,
                                                    );
                                                }
                                            }
                                        }
                                        "project" => {
                                            if let Yaml::String(value) = item_value {
                                                remote_pkg = value.clone();
                                            }
                                        }
                                        "ref" => {
                                            if let Yaml::String(value) = item_value {
                                                remote_tag = value.clone();
                                            }
                                        }
                                        "file" => {
                                            debug!("files: {:?}", item_value);
                                            if let Yaml::Array(value) = item_value {
                                                for yml in value {
                                                    if let Yaml::String(_path) = yml {
                                                        remote_files.push(_path.to_string());
                                                    }
                                                }
                                            }
                                        }
                                        _ => break,
                                    }
                                }
                            }

                            if _follow && !remote_pkg.is_empty() {
                                self.fetch_remote_files(
                                    parse_results,
                                    &remote_pkg,
                                    &remote_tag,
                                    &remote_files,
                                );
                            }
                        }
                    }
                }
            }
        }

        Some(())
    }

    fn fetch_remote_files(
        &self,
        parse_results: &mut ParseResults,
        remote_pkg: &String,
        remote_tag: &String,
        remote_files: &Vec<String>,
    ) {
        if remote_tag.is_empty() || remote_pkg.is_empty() || remote_files.is_empty() {
            return;
        }

        if let Err(err) = std::fs::create_dir_all(&self.cache_path) {
            error!("error creating cache folder; got err {}", err);

            return;
        }

        // check if we have that reference to repository
        let repo_dest = format!("{}{}/{}", &self.cache_path, remote_pkg, remote_tag);

        self.clone_repo(repo_dest.as_str(), remote_tag.as_str(), remote_pkg.as_str());

        for file in remote_files {
            let file_path = format!("{}{}", repo_dest, file);
            debug!("filepath: {}", file_path);

            let content = match std::fs::read_to_string(&file_path) {
                Ok(content) => content,
                Err(err) => {
                    error!("error reading content from: {}; got err {}", file_path, err);
                    continue;
                }
            };

            let uri = match Url::parse(format!("file://{}", &file_path).as_str()) {
                Ok(uri) => uri,
                Err(err) => {
                    error!("error generating uri; got err {}", err);
                    continue;
                }
            };

            parse_results.nodes.append(
                &mut self
                    .treesitter
                    .get_all_root_nodes(uri.as_str(), content.as_str()),
            );

            parse_results.files.push(GitlabFile {
                path: uri.as_str().into(),
                content: content.clone(),
            });

            // arrays are overriden in gitlab.
            let found_stages = self
                .treesitter
                .get_stage_definitions(uri.as_str(), content.as_str());

            if !found_stages.is_empty() {
                parse_results.stages = found_stages;
            }

            parse_results.variables.append(
                &mut self
                    .treesitter
                    .get_root_variables(uri.as_str(), content.as_str()),
            );
        }
    }

    fn clone_repo(&self, repo_dest: &str, remote_tag: &str, remote_pkg: &str) {
        // git clone --depth 1 --branch <tag_name> <repo_url>

        let repo_dest_path = std::path::Path::new(repo_dest);

        info!("repo_path: {:?}", repo_dest_path);

        if repo_dest_path.exists() {
            let mut repo_contents = match repo_dest_path.read_dir() {
                Ok(contents) => contents,
                Err(err) => {
                    error!("error reading repo contents; got err: {}", err);
                    return;
                }
            };

            if repo_contents.next().is_some() {
                info!("repo contents exist");

                return;
            }

            return;
        }

        let remotes = match self.package_map.get(remote_pkg) {
            Some(host) => vec![host.to_string()],
            None => self.remote_urls.clone(),
        };

        info!("got git host: {:?}", remotes);

        env::set_var("GIT_HTTP_LOW_SPEED_LIMIT", "1000");
        env::set_var("GIT_HTTP_LOW_SPEED_TIME", "10");

        for origin in remotes {
            match Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    "--branch",
                    remote_tag,
                    format!("{}{}", origin, remote_pkg).as_str(),
                    repo_dest,
                ])
                .output()
            {
                Ok(ok) => {
                    info!("successfully cloned to : {}; got: {:?}", repo_dest, ok);
                    break;
                }
                Err(err) => {
                    error!("error cloning to: {}, got: {:?}", repo_dest, err);

                    let dest = path::Path::new(repo_dest);
                    if dest.exists() {
                        fs::remove_dir_all(dest).expect("should be able to remove");
                    }
                    continue;
                }
            };
        }
    }
}
