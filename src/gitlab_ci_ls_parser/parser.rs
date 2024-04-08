use std::collections::HashMap;

use anyhow::anyhow;
use log::{error, info};
use lsp_types::{Position, Url};
use serde::{Deserialize, Serialize};

use super::{
    git, treesitter, GitlabElement, GitlabFile, IncludeInformation, NodeDefinition, ParseResults,
};

#[derive(Debug, Serialize, Deserialize)]
struct IncludeNode {
    include: Vec<IncludeItem>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)] // This attribute allows for different structs in the same Vec
enum IncludeItem {
    Project(Project),
    Local(Local),
    Remote(Remote),
    Basic(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(clippy::struct_field_names)]
struct Project {
    project: String,

    #[serde(rename = "ref")]
    reference: String,
    file: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Local {
    local: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Remote {
    remote: String,
}

pub trait Parser {
    fn get_all_extends(
        &self,
        uri: String,
        content: &str,
        extend_name: Option<&str>,
    ) -> Vec<GitlabElement>;
    fn get_all_job_needs(
        &self,
        uri: String,
        content: &str,
        extend_name: Option<&str>,
    ) -> Vec<GitlabElement>;
    fn get_all_stages(&self, uri: &str, content: &str, stage: Option<&str>) -> Vec<GitlabElement>;
    fn get_position_type(&self, content: &str, position: Position) -> PositionType;
    fn get_root_node(&self, uri: &str, content: &str, node_key: &str) -> Option<GitlabElement>;
    fn parse_contents(&self, uri: &Url, content: &str, _follow: bool) -> Option<ParseResults>;
    fn parse_contents_recursive(
        &self,
        parse_results: &mut ParseResults,
        uri: &lsp_types::Url,
        content: &str,
        _follow: bool,
        iteration: i32,
    ) -> Option<()>;
    fn get_variable_definitions(
        &self,
        word: &str,
        uri: &str,
        position: Position,
        store: &HashMap<String, String>,
    ) -> Option<Vec<GitlabElement>>;
    fn get_full_definition(
        &self,
        element: GitlabElement,
        store: &HashMap<String, String>,
    ) -> anyhow::Result<String>;
}

#[allow(clippy::module_name_repetitions)]
pub struct ParserImpl {
    treesitter: Box<dyn treesitter::Treesitter>,
    git: Box<dyn git::Git>,
}

// TODO: rooot for the case of importing f9
#[derive(Debug)]
pub enum PositionType {
    Extend,
    Stage,
    Variable,
    None,
    RootNode,
    Include(IncludeInformation),
    Needs(NodeDefinition),
}

impl ParserImpl {
    pub fn new(
        remote_urls: Vec<String>,
        package_map: HashMap<String, String>,
        cache_path: String,
        treesitter: Box<dyn treesitter::Treesitter>,
    ) -> ParserImpl {
        ParserImpl {
            treesitter,
            git: Box::new(git::GitImpl::new(remote_urls, package_map, cache_path)),
        }
    }

    fn merge_yaml_nodes(base: &serde_yaml::Value, other: &serde_yaml::Value) -> serde_yaml::Value {
        match (base, other) {
            // When both values are mappings, merge them.
            (serde_yaml::Value::Mapping(base_map), serde_yaml::Value::Mapping(other_map)) => {
                let mut merged_map = serde_yaml::Mapping::new();

                // Insert all values from the base map first.
                for (k, v) in base_map {
                    merged_map.insert(k.clone(), v.clone());
                }

                // Then merge or replace with values from the other map.
                for (k, v) in other_map {
                    if k == "extends" {
                        // 'extends' field in other node takes precedence.
                        merged_map.insert(k.clone(), v.clone());
                    } else if let Some(serde_yaml::Value::Sequence(_)) = base_map.get(k) {
                        // If the key is an array and exists in base, it takes precedence, do nothing.
                    } else {
                        // For all other cases, insert or replace the value from the other map.
                        merged_map.insert(
                            k.clone(),
                            ParserImpl::merge_yaml_nodes(
                                base_map.get(k).unwrap_or(&serde_yaml::Value::Null),
                                v,
                            ),
                        );
                    }
                }

                // Handle the edge case for 'extends' if it does not exist in the second node.
                if base_map.contains_key("extends") && !other_map.contains_key("extends") {
                    merged_map.remove(&serde_yaml::Value::String("extends".to_string()));
                }

                serde_yaml::Value::Mapping(merged_map)
            }
            // When values are not mappings, other takes precedence.
            (_, _) => other.clone(),
        }
    }

    fn all_nodes(
        &self,
        store: &HashMap<String, String>,
        all_nodes: &mut Vec<GitlabElement>,
        node: GitlabElement,
        iter: usize,
    ) {
        // Another safety wow
        if iter > 5 {
            return;
        }

        all_nodes.push(node.clone());

        let extends =
            self.get_all_extends(node.uri, node.content.unwrap_or_default().as_str(), None);

        if extends.is_empty() {
            return;
        }

        for extend in extends {
            for (uri, content) in store {
                if let Some(root_node) = self.get_root_node(uri, content, extend.key.as_str()) {
                    let node = GitlabElement {
                        uri: root_node.uri,
                        key: root_node.key,
                        content: Some(root_node.content.unwrap()),
                        ..Default::default()
                    };

                    self.all_nodes(store, all_nodes, node, iter + 1);

                    break;
                }
            }
        }
    }

    fn parse_remote_files(&self, parse_results: &mut ParseResults, remote_files: &[GitlabFile]) {
        for remote_file in remote_files {
            parse_results.nodes.append(
                &mut self
                    .treesitter
                    .get_all_root_nodes(remote_file.path.as_str(), remote_file.content.as_str()),
            );

            parse_results.files.push(remote_file.clone());

            // arrays are overriden in gitlab.
            let found_stages = self
                .treesitter
                .get_stage_definitions(remote_file.path.as_str(), remote_file.content.as_str());

            if !found_stages.is_empty() {
                parse_results.stages = found_stages;
            }

            parse_results.variables.append(
                &mut self
                    .treesitter
                    .get_root_variables(remote_file.path.as_str(), remote_file.content.as_str()),
            );
        }
    }

    fn parse_remote_file(&self, remote_url: &str, parse_results: &mut ParseResults) {
        let remote_url = match Url::parse(remote_url) {
            Ok(f) => f,
            Err(err) => {
                error!(
                    "could not parse remote URL: {}; got err: {:?}",
                    remote_url, err
                );

                return;
            }
        };
        let file = match self.git.fetch_remote(remote_url.clone()) {
            Ok(res) => res,
            Err(err) => {
                error!(
                    "error retrieving remote file: {}; got err: {:?}",
                    remote_url, err
                );

                return;
            }
        };

        self.parse_remote_files(parse_results, &[file]);
    }

    fn parse_local_file(
        &self,
        uri: &Url,
        local_url: &str,
        follow: bool,
        parse_results: &mut ParseResults,
        iteration: i32,
    ) -> Option<()> {
        let current_uri = uri.join(local_url).ok()?;
        let current_content = std::fs::read_to_string(current_uri.path()).ok()?;
        if follow {
            self.parse_contents_recursive(
                parse_results,
                &current_uri,
                &current_content,
                follow,
                iteration + 1,
            );
        };
        Some(())
    }
}

impl Parser for ParserImpl {
    fn get_all_extends(
        &self,
        uri: String,
        content: &str,
        extend_name: Option<&str>,
    ) -> Vec<GitlabElement> {
        self.treesitter.get_all_extends(uri, content, extend_name)
    }

    fn get_all_stages(&self, uri: &str, content: &str, stage: Option<&str>) -> Vec<GitlabElement> {
        self.treesitter.get_all_stages(uri, content, stage)
    }

    fn get_position_type(&self, content: &str, position: Position) -> PositionType {
        self.treesitter.get_position_type(content, position)
    }

    fn get_root_node(&self, uri: &str, content: &str, node_key: &str) -> Option<GitlabElement> {
        self.treesitter.get_root_node(uri, content, node_key)
    }

    fn parse_contents(&self, uri: &Url, content: &str, follow: bool) -> Option<ParseResults> {
        let files: Vec<GitlabFile> = vec![];
        let nodes: Vec<GitlabElement> = vec![];
        let stages: Vec<GitlabElement> = vec![];
        let variables: Vec<GitlabElement> = vec![];

        let mut parse_results = ParseResults {
            files,
            nodes,
            stages,
            variables,
        };

        self.parse_contents_recursive(&mut parse_results, uri, content, follow, 0)?;

        Some(parse_results)
    }

    #[allow(clippy::too_many_lines)]
    fn parse_contents_recursive(
        &self,
        parse_results: &mut ParseResults,
        uri: &lsp_types::Url,
        content: &str,
        follow: bool,
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

        if let Some(element) = self
            .treesitter
            .get_root_node(uri.as_str(), content, "include")
        {
            let include_node: IncludeNode = match serde_yaml::from_str(&element.content.clone()?) {
                Ok(y) => y,
                Err(err) => {
                    error!(
                        "error parsing yaml: {}, got err: {}",
                        &element.content?, err
                    );

                    return None;
                }
            };

            for include_node in include_node.include {
                match include_node {
                    IncludeItem::Local(node) => {
                        self.parse_local_file(uri, &node.local, follow, parse_results, iteration)?;
                    }
                    IncludeItem::Remote(node) => {
                        self.parse_remote_file(&node.remote, parse_results);
                    }
                    IncludeItem::Basic(include_url) => {
                        if let Ok(url) = Url::parse(&include_url) {
                            info!("got remote URL: {url}");
                            self.parse_remote_file(url.as_str(), parse_results);
                        } else {
                            info!("got local URL: {include_url}");
                            self.parse_local_file(
                                uri,
                                &include_url,
                                follow,
                                parse_results,
                                iteration,
                            )?;
                        }
                    }
                    IncludeItem::Project(node) => {
                        let remote_files = match self.git.fetch_remote_repository(
                            node.project.as_str(),
                            node.reference.as_str(),
                            node.file,
                        ) {
                            Ok(rf) => rf,
                            Err(err) => {
                                error!("error retrieving remote files: {}", err);

                                vec![]
                            }
                        };

                        self.parse_remote_files(parse_results, &remote_files);
                    }
                }
            }
        }

        Some(())
    }

    fn get_all_job_needs(
        &self,
        uri: String,
        content: &str,
        needs_name: Option<&str>,
    ) -> Vec<GitlabElement> {
        self.treesitter.get_all_job_needs(uri, content, needs_name)
    }

    fn get_variable_definitions(
        &self,
        variable: &str,
        uri: &str,
        position: Position,
        store: &HashMap<String, String>,
    ) -> Option<Vec<GitlabElement>> {
        let mut all_nodes = vec![];

        if let Some(content) = store.get(uri) {
            let element = self
                .treesitter
                .get_root_node_at_position(content, position)?;

            self.all_nodes(store, &mut all_nodes, element, 0);
        }

        Some(
            all_nodes
                .iter()
                .filter_map(|e| {
                    let cnt = store.get(&e.uri)?;
                    self.treesitter
                        .job_variable_definition(e.uri.as_str(), cnt, variable, &e.key)
                })
                .collect(),
        )
    }

    fn get_full_definition(
        &self,
        element: GitlabElement,
        store: &HashMap<String, String>,
    ) -> anyhow::Result<String> {
        let mut all_nodes = vec![];

        self.all_nodes(store, &mut all_nodes, element.clone(), 0);

        let init = serde_yaml::from_str("")
            .map_err(|e| anyhow!("error initializing empty yaml node; got err: {e}"))?;

        let merged = all_nodes
            .iter()
            .filter_map(|n| n.content.clone())
            .map(|c| c.lines().skip(1).collect::<Vec<&str>>().join("\n"))
            .fold(init, |acc, x| {
                let current = serde_yaml::from_str(x.as_str());

                if let Ok(curr) = current {
                    ParserImpl::merge_yaml_nodes(&acc, &curr)
                } else {
                    acc
                }
            });

        let mut top_level_map = serde_yaml::Mapping::new();
        top_level_map.insert(serde_yaml::Value::String(element.key), merged);

        let merged_with_key = serde_yaml::Value::Mapping(top_level_map);

        serde_yaml::to_string(&merged_with_key)
            .map_err(|e| anyhow!("error serializing node; got err: {e}"))
    }
}
