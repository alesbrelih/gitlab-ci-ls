use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
};

use anyhow::anyhow;
use log::{error, info, warn};
use lsp_types::{Position, Url};

use super::{
    fs_utils, git, parser_utils::ParserUtils, treesitter, Component, ComponentSpec,
    GitlabCacheElement, GitlabComponentElement, GitlabElement, GitlabElementWithParentAndLvl,
    GitlabFile, GitlabFileElements, IncludeInformation, IncludeItem, IncludeNode, LSPConfig,
    NodeDefinition, ParseResults, RuleReference,
};

unsafe impl Sync for ParserImpl {}

pub trait Parser: Sync {
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
    fn get_all_rule_references(
        &self,
        uri: String,
        content: &str,
        rule_name: Option<&str>,
    ) -> Vec<GitlabElement>;
    fn get_all_components(&self, uri: &str, content: &str) -> Vec<GitlabComponentElement>;
    fn get_all_multi_caches(&self, uri: &str, content: &str) -> Vec<GitlabCacheElement>;
    fn get_all_stages(&self, uri: &str, content: &str, stage: Option<&str>) -> Vec<GitlabElement>;
    fn get_position_type(&self, content: &str, position: Position) -> PositionType;
    fn get_root_node(&self, uri: &str, content: &str, node_key: &str) -> Option<GitlabElement>;
    fn get_root_node_key(&self, uri: &str, content: &str, node_key: &str) -> Option<GitlabElement>;
    fn get_root_node_at_position(&self, content: &str, position: Position)
        -> Option<GitlabElement>;
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
        node_list: &[GitlabFileElements],
    ) -> Option<Vec<GitlabElement>>;
    fn get_full_definition(
        &self,
        element: GitlabElement,
        node_list: &[GitlabFileElements],
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
    Dependency,
    Include(IncludeInformation),
    Needs(NodeDefinition),
    RuleReference(RuleReference),
}

impl ParserImpl {
    pub fn new(
        remote_urls: Vec<String>,
        package_map: HashMap<String, String>,
        cache_path: String,
        treesitter: Box<dyn treesitter::Treesitter>,
        fs_utils: Box<dyn fs_utils::FSUtils>,
    ) -> ParserImpl {
        ParserImpl {
            treesitter,
            git: Box::new(git::GitImpl::new(
                remote_urls,
                package_map,
                cache_path,
                fs_utils,
            )),
        }
    }

    fn merge_yaml_nodes(
        base: &serde_yaml::Value,
        base_parents: &str,
        other: &serde_yaml::Value,
        other_parents: &str,
    ) -> serde_yaml::Value {
        match (base, other) {
            // Merge mappings
            (serde_yaml::Value::Mapping(base_map), serde_yaml::Value::Mapping(other_map)) => {
                let mut merged_map = serde_yaml::Mapping::new();

                let (primary, secondary, primary_parents, secondary_parents) =
                    if other_parents.contains(base_parents) {
                        (base_map, other_map, base_parents, other_parents)
                    } else {
                        (other_map, base_map, other_parents, base_parents)
                    };

                // Insert values from primary map
                for (k, v) in primary {
                    if k != "extends" {
                        merged_map.insert(k.clone(), v.clone());
                    }
                }

                // Merge or replace values from secondary map
                for (k, v) in secondary {
                    if k == "extends" {
                        continue;
                    }
                    if let Some(serde_yaml::Value::Sequence(_)) = primary.get(k) {
                        // Skip if key is an array in primary
                        continue;
                    }
                    merged_map.insert(
                        k.clone(),
                        Self::merge_yaml_nodes(
                            primary.get(k).unwrap_or(&serde_yaml::Value::Null),
                            primary_parents,
                            v,
                            secondary_parents,
                        ),
                    );
                }

                serde_yaml::Value::Mapping(merged_map)
            }
            // Base takes precedence unless null
            (_, _) => match (base.is_null(), other.is_null()) {
                (true, false) => other.clone(),
                (false, true) => base.clone(),
                _ => {
                    if other_parents.contains(base_parents) {
                        base.clone()
                    } else {
                        other.clone()
                    }
                }
            },
        }
    }

    fn calculate_hash<T: Hash>(t: &T) -> u64 {
        let mut s = DefaultHasher::new();
        t.hash(&mut s);
        s.finish()
    }

    fn get_all_nodes(
        &self,
        node_list: &[GitlabFileElements],
        all_nodes: &mut Vec<GitlabElementWithParentAndLvl>,
        node: GitlabElementWithParentAndLvl,
    ) {
        // Another safety wow
        if node.lvl > 5 {
            return;
        }

        all_nodes.push(node.clone());

        // check if we find another job that was named the same way
        // to prevent recursion we can check object hash to not match original job hash
        // that means it's a different job
        for file in node_list {
            for n in &file.elements {
                if n.key == node.el.key
                    && !all_nodes
                        .iter()
                        .any(|e| Self::calculate_hash(&e.el) == Self::calculate_hash(&n))
                {
                    let el = GitlabElementWithParentAndLvl {
                        el: n.clone(),
                        lvl: node.lvl,
                        parents: node.parents.clone(),
                    };
                    self.get_all_nodes(node_list, all_nodes, el);
                }
            }
        }

        let extends = self.get_all_extends(
            node.el.uri,
            node.el.content.unwrap_or_default().as_str(),
            None,
        );

        if extends.is_empty() {
            return;
        }

        for extend in extends {
            for file in node_list {
                for n in &file.elements {
                    if n.key == extend.key {
                        let el = GitlabElementWithParentAndLvl {
                            el: n.clone(),
                            lvl: node.lvl + 1,
                            parents: format!("{}-{}", node.parents.clone(), extend.key),
                        };
                        self.get_all_nodes(node_list, all_nodes, el);
                    }
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
                error!("could not parse remote URL: {remote_url}; got err: {err:?}");

                return;
            }
        };
        let file = match self.git.fetch_remote(remote_url.clone()) {
            Ok(res) => res,
            Err(err) => {
                error!("error retrieving remote file: {remote_url}; got err: {err:?}");

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
        if follow {
            if ParserUtils::is_glob(local_url) {
                let files = ParserUtils::gitlab_style_glob(local_url);
                for f in files {
                    let current_uri = uri.join(f.to_str()?).ok()?;
                    self.parse_local_file(
                        uri,
                        current_uri.as_str(),
                        follow,
                        parse_results,
                        iteration,
                    );
                }
            } else {
                let current_uri = uri.join(local_url).ok()?;
                let current_content = std::fs::read_to_string(current_uri.path()).ok()?;
                self.parse_contents_recursive(
                    parse_results,
                    &current_uri,
                    &current_content,
                    follow,
                    iteration + 1,
                );
            }
        }
        Some(())
    }

    fn parse_component(
        &self,
        parse_results: &mut ParseResults,
        component_id: &str,
    ) -> anyhow::Result<()> {
        let component_info = match ParserUtils::extract_component_from_uri(
            component_id,
            self.git.get_project_remote_uris(),
        ) {
            Ok(c) => c,
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "error extracting component info from uri; got: {err}"
                ));
            }
        };

        let gitlab_component = match self.git.fetch_remote_component(component_info.clone()) {
            Ok(c) => c,
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "could not find gitlab component: {:?}; got err: {err}",
                    component_info
                ));
            }
        };

        let p = &gitlab_component.uri["file://".len()..];
        let spec_content = match std::fs::read_to_string(p) {
            Ok(s) => s,
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "could not read gitlab component: {:?}, got err: {err}",
                    p,
                ));
            }
        };

        // TODO: probably it is valid to have no inputs?
        let Some(spec_inputs) = self.treesitter.get_component_spec_inputs(&spec_content) else {
            return Err(anyhow::anyhow!("could not get spec inputs from component"));
        };

        let spec: ComponentSpec = match serde_yaml::from_str(&spec_inputs) {
            Ok(y) => y,
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "error parsing component spec yaml: {}, got err: {}",
                    &spec_content,
                    err
                ));
            }
        };

        parse_results.components.push(Component {
            uri: component_id.to_string(),
            local_path: gitlab_component.uri,
            inputs: spec
                .spec
                .inputs
                .into_iter()
                .map(|i| crate::gitlab_ci_ls_parser::ComponentInput {
                    key: i.0,
                    default: i.1.default,
                    regex: i.1.regex,
                    options: i.1.options,
                    prop_type: i.1.type_,
                    description: i.1.description,

                    ..Default::default()
                })
                .collect(),
        });

        Ok(())
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

    fn get_all_components(&self, uri: &str, content: &str) -> Vec<GitlabComponentElement> {
        self.treesitter.get_all_components(uri, content)
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
        let components: Vec<Component> = vec![];
        let variables: Vec<GitlabElement> = vec![];

        let mut parse_results = ParseResults {
            files,
            nodes,
            stages,
            components,
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

                    return Some(());
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
                            node.reference.as_deref(),
                            node.file,
                        ) {
                            Ok(rf) => rf,
                            Err(err) => {
                                error!("error retrieving remote files: {err}");

                                vec![]
                            }
                        };

                        self.parse_remote_files(parse_results, &remote_files);
                    }
                    IncludeItem::Component(node) => {
                        if let Err(err) = self.parse_component(parse_results, &node.component) {
                            error!("error handling component; got err: {err}");
                        }
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

    fn get_all_rule_references(
        &self,
        uri: String,
        content: &str,
        rule_name: Option<&str>,
    ) -> Vec<GitlabElement> {
        self.treesitter
            .get_all_rule_references(&uri, content, rule_name)
    }

    fn get_variable_definitions(
        &self,
        variable: &str,
        uri: &str,
        position: Position,
        store: &HashMap<String, String>,
        node_list: &[GitlabFileElements],
    ) -> Option<Vec<GitlabElement>> {
        let mut all_nodes = vec![];

        if let Some(content) = store.get(uri) {
            let element = self
                .treesitter
                .get_root_node_at_position(content, position)?;

            let el = GitlabElementWithParentAndLvl {
                el: element,
                lvl: 0,
                parents: "root".to_string(),
            };

            self.get_all_nodes(node_list, &mut all_nodes, el);
        }

        Some(
            all_nodes
                .iter()
                .filter_map(|e| {
                    let cnt = store.get(&e.el.uri)?;
                    self.treesitter.job_variable_definition(
                        e.el.uri.as_str(),
                        cnt,
                        variable,
                        &e.el.key,
                    )
                })
                .collect(),
        )
    }

    fn get_full_definition(
        &self,
        top_node: GitlabElement,
        node_list: &[GitlabFileElements],
    ) -> anyhow::Result<String> {
        struct MergeNode {
            yaml: serde_yaml::Value,
            parents: String,
        }

        let mut all_nodes: Vec<GitlabElementWithParentAndLvl> = Vec::new();

        let root_node = GitlabElementWithParentAndLvl {
            el: top_node.clone(),
            lvl: 0,
            parents: "root".to_string(),
        };

        self.get_all_nodes(node_list, &mut all_nodes, root_node);

        if let Some(default) = node_list
            .iter()
            .flat_map(|e| &e.elements)
            .find(|e| e.key == "default")
        {
            all_nodes.push(GitlabElementWithParentAndLvl {
                el: default.clone(),
                lvl: 999, // Defaults have the lowest priority
                parents: "root".to_string(),
            });
        }

        let init_node = MergeNode {
            yaml: serde_yaml::from_str("")
                .map_err(|e| anyhow!("Error initializing empty YAML node: {e}"))?,
            parents: "root".to_string(),
        };

        let mut merged = all_nodes.iter().fold(init_node, |acc, x| {
            let content = x.el.content.as_deref().unwrap_or("");
            let current_content = content.lines().skip(1).collect::<Vec<_>>().join("\n");

            match serde_yaml::from_str(&current_content) {
                Ok(current_yaml) => MergeNode {
                    yaml: ParserImpl::merge_yaml_nodes(
                        &acc.yaml,
                        &acc.parents,
                        &current_yaml,
                        &x.parents,
                    ),
                    parents: x.parents.clone(),
                },
                Err(_) => acc,
            }
        });

        if let Some(content) = top_node.content {
            let current_content = content.lines().skip(1).collect::<Vec<_>>().join("\n");
            if let Ok(current_yaml) = serde_yaml::from_str(&current_content) {
                merged = MergeNode {
                    yaml: ParserImpl::merge_yaml_nodes(
                        &merged.yaml,
                        &merged.parents,
                        &current_yaml,
                        "overwrite", // setting different parent so its get precedence over
                                     // previous content
                    ),
                    parents: "overwrite".to_string(),
                };
            }
        }

        let mut top_level_map = serde_yaml::Mapping::new();
        top_level_map.insert(serde_yaml::Value::String(top_node.key), merged.yaml);

        let final_yaml = serde_yaml::Value::Mapping(top_level_map);

        serde_yaml::to_string(&final_yaml).map_err(|e| anyhow!("Error serializing node: {e}"))
    }

    fn get_root_node_key(&self, uri: &str, content: &str, node_key: &str) -> Option<GitlabElement> {
        self.treesitter.get_root_node_key(uri, content, node_key)
    }

    fn get_all_multi_caches(&self, uri: &str, content: &str) -> Vec<GitlabCacheElement> {
        self.treesitter.get_all_multi_caches(uri, content)
    }

    fn get_root_node_at_position(
        &self,
        content: &str,
        position: Position,
    ) -> Option<GitlabElement> {
        self.treesitter.get_root_node_at_position(content, position)
    }
}

#[cfg(test)]
mod tests {
    use fs_utils::MockFSUtils;
    use treesitter::TreesitterImpl;

    use super::*;

    #[allow(clippy::too_many_lines)]
    #[test]
    fn test_get_all_nodes() {
        let parser = ParserImpl::new(
            vec![],
            HashMap::new(),
            String::new(),
            Box::new(TreesitterImpl::new()),
            Box::new(MockFSUtils::new()),
        );

        let first_content = r"
        .first:
          image: alpine
          extends: .base
        ";

        let second_content = r#"
        .second:
          image: centos
          extends:
            - .minimal
          variables:
            JUST: "kidding"
        "#;

        let minimal_content = r#"
        .minimal:
          before_script: "hello there"
        "#;

        let base_content = r#"
        .base:
          variables:
            LOREM: "ipsum"
            IPSUM: "lorem"
        "#;

        let job_content = r#"
        job:
          extends:
            - .first
            - .second
          variables:
            LOREM: "job"
        "#;

        let duplicated_job_content = r"
        job:
          image: ubuntu
        ";

        let other_job_content = r"
        other_job:
          extends:
            - .other
          image: golang
        ";

        let other_base_content = r#"
        .other:
          variables:
            BASE: "other"
        "#;

        let first = GitlabElement {
            key: ".first".to_string(),
            content: Some(first_content.to_string()),
            ..Default::default()
        };

        let second = GitlabElement {
            key: ".second".to_string(),
            content: Some(second_content.to_string()),
            ..Default::default()
        };

        let base = GitlabElement {
            key: ".base".to_string(),
            content: Some(base_content.to_string()),
            ..Default::default()
        };

        let minimal = GitlabElement {
            key: ".minimal".to_string(),
            content: Some(minimal_content.to_string()),
            ..Default::default()
        };

        let job = GitlabElement {
            key: "job".to_string(),
            content: Some(job_content.to_string()),
            ..Default::default()
        };

        let duplicated = GitlabElement {
            key: "job".to_string(),
            content: Some(duplicated_job_content.to_string()),
            ..Default::default()
        };

        let other_job = GitlabElement {
            key: "other_job".to_string(),
            content: Some(other_job_content.to_string()),
            ..Default::default()
        };

        let other = GitlabElement {
            key: ".other".to_string(),
            content: Some(other_base_content.to_string()),
            ..Default::default()
        };

        let mocked_node_list: Vec<GitlabFileElements> = vec![
            GitlabFileElements {
                uri: "first-file.yml".to_string(),
                elements: vec![
                    duplicated.clone(),
                    first.clone(),
                    base.clone(),
                    other_job.clone(),
                    other.clone(),
                    minimal.clone(),
                ],
            },
            GitlabFileElements {
                uri: "second-file.yml".to_string(),
                elements: vec![second.clone()],
            },
        ];

        let initial_node = GitlabElementWithParentAndLvl {
            el: job.clone(),
            lvl: 0,
            parents: "root".to_string(),
        };

        let mut all_nodes: Vec<GitlabElementWithParentAndLvl> = vec![];
        parser.get_all_nodes(&mocked_node_list, &mut all_nodes, initial_node);

        assert_eq!(all_nodes.len(), 6);

        let want: Vec<GitlabElementWithParentAndLvl> = vec![
            GitlabElementWithParentAndLvl {
                lvl: 0,
                parents: "root".to_string(),
                el: job.clone(),
            },
            GitlabElementWithParentAndLvl {
                lvl: 0,
                parents: "root".to_string(),
                el: duplicated.clone(),
            },
            GitlabElementWithParentAndLvl {
                lvl: 1,
                parents: "root-.first".to_string(),
                el: first.clone(),
            },
            GitlabElementWithParentAndLvl {
                lvl: 2,
                parents: "root-.first-.base".to_string(),
                el: base.clone(),
            },
            GitlabElementWithParentAndLvl {
                lvl: 1,
                parents: "root-.second".to_string(),
                el: second.clone(),
            },
            GitlabElementWithParentAndLvl {
                lvl: 2,
                parents: "root-.second-.minimal".to_string(),
                el: minimal,
            },
        ];

        for (idx, el) in all_nodes.iter().enumerate() {
            assert_eq!(el, &want[idx]);
        }
    }

    #[allow(clippy::too_many_lines)]
    #[test]
    fn test_get_full_definition() {
        let parser = ParserImpl::new(
            vec![],
            HashMap::new(),
            String::new(),
            Box::new(TreesitterImpl::new()),
            Box::new(MockFSUtils::new()),
        );

        let first_content = r".first:
  image: alpine
  extends: .base
";

        let second_content = r#".second:
  image: centos
  extends:
    - .minimal
  variables:
    JUST: "kidding"
"#;

        let minimal_content = r#".minimal:
  before_script: "hello there"
"#;

        let base_content = r#".base:
  variables:
    LOREM: "ipsum"
    IPSUM: "lorem"
"#;

        let job_content = r#"job:
  extends:
    - .first
    - .second
  variables:
    LOREM: "job"
"#;

        let duplicated_job_content = r"job:
  image: ubuntu
  script: hi job
";

        let other_job_content = r"other_job:
  image: golang
  extends:
    - .other
";

        let other_base_content = r#".other:
  variables:
    BASE: "other"
"#;

        let first = GitlabElement {
            key: ".first".to_string(),
            content: Some(first_content.to_string()),
            ..Default::default()
        };

        let second = GitlabElement {
            key: ".second".to_string(),
            content: Some(second_content.to_string()),
            ..Default::default()
        };

        let base = GitlabElement {
            key: ".base".to_string(),
            content: Some(base_content.to_string()),
            ..Default::default()
        };

        let minimal = GitlabElement {
            key: ".minimal".to_string(),
            content: Some(minimal_content.to_string()),
            ..Default::default()
        };

        let job = GitlabElement {
            key: "job".to_string(),
            content: Some(job_content.to_string()),
            ..Default::default()
        };

        let duplicated = GitlabElement {
            key: "job".to_string(),
            content: Some(duplicated_job_content.to_string()),
            ..Default::default()
        };

        let other_job = GitlabElement {
            key: "other_job".to_string(),
            content: Some(other_job_content.to_string()),
            ..Default::default()
        };

        let other = GitlabElement {
            key: ".other".to_string(),
            content: Some(other_base_content.to_string()),
            ..Default::default()
        };

        let mocked_node_list: Vec<GitlabFileElements> = vec![
            GitlabFileElements {
                uri: "first-file.yml".to_string(),
                elements: vec![
                    duplicated.clone(),
                    first.clone(),
                    base.clone(),
                    other_job.clone(),
                    other.clone(),
                    minimal.clone(),
                ],
            },
            GitlabFileElements {
                uri: "second-file.yml".to_string(),
                elements: vec![second.clone()],
            },
        ];

        let full_definition = parser.get_full_definition(job.clone(), &mocked_node_list);

        assert!(full_definition.is_ok());

        let want = r"job:
  variables:
    LOREM: job
    IPSUM: lorem
    JUST: kidding
  image: ubuntu
  script: hi job
  before_script: hello there
";

        assert_eq!(full_definition.unwrap(), want);
    }
}
