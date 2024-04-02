use std::{collections::HashMap, env, fs, path, process::Command};

use log::{debug, error, info};
use lsp_types::{Position, Url};
use tree_sitter::{Query, QueryCursor};
use tree_sitter_yaml::language;
use yaml_rust::{Yaml, YamlLoader};

use crate::{GitlabElement, GitlabFile, GitlabRootNode, LSPPosition, ParseResults, Range};

pub struct Parser {
    cache_path: String,
    package_map: HashMap<String, String>,
    remote_urls: Vec<String>,
}

// TODO: rooot for the case of importing f9
pub enum CompletionType {
    Extend,
    Stage,
    Variable,
    None,
    RootNode,
}

pub struct ParserUtils {}

impl ParserUtils {
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

    #[allow(dead_code)]
    pub fn find_position(content: &str, word: &str) -> Option<Range> {
        for (line_num, line) in content.lines().enumerate() {
            if line.starts_with(word) {
                return Some(Range {
                    start: LSPPosition {
                        line: line_num as u32,
                        character: 0,
                    },
                    end: LSPPosition {
                        line: line_num as u32,
                        character: word.len() as u32,
                    },
                });
            }
        }

        None
    }

    // Returns key and full node
    pub fn get_root_node(uri: &str, content: &str, node_key: &str) -> Option<GitlabElement> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(tree_sitter_yaml::language())
            .expect("Error loading YAML grammar");

        let query_source = format!(
            r#"
        (
            stream(
                document(
                    block_node(
                        block_mapping(
                            block_mapping_pair
                                key: (flow_node(plain_scalar(string_scalar)@key))
                        )@value
                    )
                )
            )
            (#eq? @key "{}")
        )
        "#,
            node_key
        );

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(language(), query_source.as_str()).unwrap();
        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        for mat in matches.into_iter() {
            for c in mat.captures {
                if c.index == 1 {
                    let text = &content[c.node.byte_range()];

                    return Some(GitlabElement {
                        uri: uri.to_string(),
                        key: node_key.to_string(),
                        content: Some(text.to_string()),
                        range: Range {
                            start: LSPPosition {
                                line: c.node.start_position().row as u32,
                                character: c.node.start_position().column as u32,
                            },
                            end: LSPPosition {
                                line: c.node.end_position().row as u32,
                                character: c.node.end_position().column as u32,
                            },
                        },
                    });
                }
            }
        }

        None
    }

    pub fn get_all_root_nodes(uri: &str, content: &str) -> Vec<GitlabRootNode> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(tree_sitter_yaml::language())
            .expect("Error loading YAML grammar");

        let query_source = r#"
        (
            stream(
                document(
                block_node(
                    block_mapping(
                    block_mapping_pair
                        key: (flow_node(plain_scalar(string_scalar)@key))
                    )@value
                )
                )
            )
        )
        "#;

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(language(), query_source).unwrap();

        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut root_nodes = vec![];
        for m in matches.into_iter() {
            let mut node = GitlabRootNode {
                uri: uri.to_string(),
                ..Default::default()
            };
            for c in m.captures {
                let text = content[c.node.byte_range()].to_string();
                match c.index {
                    0 => {
                        node.key = text;
                    }
                    1 => {
                        node.description = text;
                    }
                    _ => {}
                }
            }

            root_nodes.push(node);
        }

        root_nodes
    }

    pub fn get_root_variables(uri: &str, content: &str) -> Vec<GitlabElement> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(tree_sitter_yaml::language())
            .expect("Error loading YAML grammar");

        let query_source = r#"
        (
            stream(
                document(
                    block_node(
                        block_mapping(
                            block_mapping_pair
                                key: (flow_node(plain_scalar(string_scalar) @key))
                                value: (block_node(
                                    block_mapping(
                                        block_mapping_pair
                                            key: (flow_node(plain_scalar(string_scalar)@env_key))
                                    )
                                )
                            )
                        )
                    )
                )
            )
        (#eq? @key "variables")
        )
        "#;

        // TODO: this should be generic fn accepting treesitter query

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(language(), query_source).unwrap();
        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut environments = vec![];
        for mat in matches.into_iter() {
            for c in mat.captures {
                if c.index == 1 {
                    let text = &content[c.node.byte_range()];
                    if c.node.start_position().row != c.node.end_position().row {
                        // sanity check
                        error!(
                            "environemnt spans over multiple rows: uri: {} text: {}",
                            uri, text
                        );

                        continue;
                    }

                    environments.push(GitlabElement {
                        key: text.to_owned(),
                        content: None,
                        uri: uri.to_string(),
                        range: Range {
                            start: LSPPosition {
                                line: c.node.start_position().row as u32,
                                character: c.node.start_position().column as u32,
                            },
                            end: LSPPosition {
                                line: c.node.end_position().row as u32,
                                character: c.node.end_position().column as u32,
                            },
                        },
                    });
                }
            }
        }

        environments
    }

    pub fn get_stage_definitions(uri: &str, content: &str) -> Vec<GitlabElement> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(tree_sitter_yaml::language())
            .expect("Error loading YAML grammar");

        let query_source = r#"
        (
            block_mapping_pair
            key: (flow_node(plain_scalar(string_scalar) @key))
            value: (block_node(block_sequence(block_sequence_item(flow_node(plain_scalar(string_scalar) @value)))))

            (#eq? @key "stages")
        )
        "#;

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(language(), query_source).unwrap();
        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut stages = vec![];
        for mat in matches.into_iter() {
            for c in mat.captures {
                if c.index == 1 {
                    let text = &content[c.node.byte_range()];
                    if c.node.start_position().row != c.node.end_position().row {
                        // sanity check
                        error!(
                            "STAGE: extends spans over multiple rows: uri: {} text: {}",
                            uri, text
                        );

                        continue;
                    }

                    stages.push(GitlabElement {
                        key: text.to_owned(),
                        content: None,
                        uri: uri.to_string(),
                        range: Range {
                            start: LSPPosition {
                                line: c.node.start_position().row as u32,
                                character: c.node.start_position().column as u32,
                            },
                            end: LSPPosition {
                                line: c.node.end_position().row as u32,
                                character: c.node.end_position().column as u32,
                            },
                        },
                    });
                }
            }
        }

        stages
    }

    pub fn get_all_stages(uri: String, content: &str) -> Vec<GitlabElement> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(tree_sitter_yaml::language())
            .expect("Error loading YAML grammar");

        let query_source = r#"
        (
            block_mapping_pair
                key: (
                    flow_node(
                        plain_scalar(string_scalar) @key
                    )
                )
                value: (
                    flow_node(
                        plain_scalar(string_scalar) @value
                    )
                )
            (#eq? @key "stage")
        )
        "#;

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(language(), query_source).unwrap();
        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut extends: Vec<GitlabElement> = vec![];

        let valid_indexes: Vec<u32> = vec![1, 2];
        for mat in matches.into_iter() {
            for c in mat.captures {
                if valid_indexes.contains(&c.index) {
                    let text = &content[c.node.byte_range()];
                    if c.node.start_position().row != c.node.end_position().row {
                        // sanity check
                        error!(
                            "ALL STAGES: extends spans over multiple rows: uri: {} text: {}",
                            uri, text
                        );

                        continue;
                    }

                    extends.push(GitlabElement {
                        key: text.to_owned(),
                        content: None,
                        uri: uri.clone(),
                        range: Range {
                            start: LSPPosition {
                                line: c.node.start_position().row as u32,
                                character: c.node.start_position().column as u32,
                            },
                            end: LSPPosition {
                                line: c.node.end_position().row as u32,
                                character: c.node.end_position().column as u32,
                            },
                        },
                    });
                }
            }
        }

        extends
    }

    pub fn get_all_extends(
        uri: String,
        content: &str,
        extend_name: Option<&str>,
    ) -> Vec<GitlabElement> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(tree_sitter_yaml::language())
            .expect("Error loading YAML grammar");

        let mut search = "".to_string();
        if extend_name.is_some() {
            search = format!("(#eq? @value \"{}\")", extend_name.unwrap());
        }

        let query_source = format!(
            r#"
        (
            block_mapping_pair
            key: (flow_node) @key
            value: [
                (flow_node(plain_scalar(string_scalar))) @value
                (block_node(block_sequence(block_sequence_item(flow_node(plain_scalar(string_scalar) @value)))))
            ]
            (#eq? @key "extends")
            {}
        )
        "#,
            search
        );

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(language(), &query_source).unwrap();
        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut extends: Vec<GitlabElement> = vec![];

        for mat in matches.into_iter() {
            for c in mat.captures {
                if c.index == 1 {
                    let text = &content[c.node.byte_range()];
                    if c.node.start_position().row != c.node.end_position().row {
                        // sanity check
                        error!(
                            "ALL: extends spans over multiple rows: uri: {} text: {}",
                            uri, text
                        );

                        continue;
                    }

                    extends.push(GitlabElement {
                        key: text.to_owned(),
                        content: None,
                        uri: uri.clone(),
                        range: Range {
                            start: LSPPosition {
                                line: c.node.start_position().row as u32,
                                character: c.node.start_position().column as u32,
                            },
                            end: LSPPosition {
                                line: c.node.end_position().row as u32,
                                character: c.node.end_position().column as u32,
                            },
                        },
                    });
                }
            }
        }

        extends
    }

    pub fn get_position_type(content: &str, position: Position) -> CompletionType {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(tree_sitter_yaml::language())
            .expect("Error loading YAML grammar");

        // 1. extends
        // 2. stages
        // 3. image variables
        // 4. before_script variables
        let query_source = r#"

            (
                block_mapping_pair
                key: (flow_node) @keyextends
                value: [
                    (flow_node(plain_scalar(string_scalar))) @extends
                    (block_node(block_sequence(block_sequence_item) @extends))
                ]
                (#eq? @keyextends "extends")
            )
            (
                block_mapping_pair
                    key: (
                        flow_node(
                            plain_scalar(string_scalar) @keystage
                        )
                    )
                    value: (
                        flow_node(
                            plain_scalar(string_scalar) @stage
                        )
                    )
                (#eq? @keystage "stage")
            )
            (
                block_mapping_pair
                key: (
                    flow_node(
                        plain_scalar(string_scalar)  @keyvariable
                    )
                )
                value:
                [
                    (
                        block_node(
                            block_mapping(block_mapping_pair
                                value:
                                    [
                                        (flow_node(flow_sequence(flow_node(double_quote_scalar)) ))
                                        (flow_node(double_quote_scalar))
                                    ] @variable
                            )
                        )
                    )
                    (
                        block_node(
                            block_sequence(block_sequence_item(flow_node(plain_scalar(string_scalar)))) @variable
                        )
                    )
                    (
                        block_node(
                            block_sequence(
                                block_sequence_item(
                                    block_node(
                                        block_mapping(
                                            block_mapping_pair
                                            value: (flow_node(double_quote_scalar)) @variable
                                        )
                                    )
                                )
                            )
                        )
                    )
                ]
            (#any-of? @keyvariable "image" "before_script" "after_script" "script" "rules" "variables")
            )
            (
                stream(
                    document(
                        block_node(
                            block_mapping(
                                block_mapping_pair
                                    key: (flow_node(plain_scalar(string_scalar)@rootnode))
                            )
                        )
                    )
                )
            )
        "#;

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(language(), query_source).unwrap();
        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        for mat in matches.into_iter() {
            for c in mat.captures {
                if c.node.start_position().row <= position.line as usize
                    && c.node.end_position().row >= position.line as usize
                {
                    match c.index {
                        0 => continue,
                        1 => return CompletionType::Extend,
                        2 => continue,
                        3 => return CompletionType::Stage,
                        4 => continue,
                        5 => return CompletionType::Variable,
                        6 => return CompletionType::RootNode,
                        _ => {
                            error!("invalid index: {}", c.index);
                            CompletionType::None
                        }
                    };
                }
            }
        }

        CompletionType::None
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
        }
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
            .append(&mut ParserUtils::get_all_root_nodes(uri.as_str(), content));

        parse_results
            .variables
            .append(&mut ParserUtils::get_root_variables(uri.as_str(), content));

        // arrays are overriden in gitlab.
        let found_stages = ParserUtils::get_stage_definitions(uri.as_str(), content);
        if !found_stages.is_empty() {
            parse_results.stages = found_stages;
        }

        let element = ParserUtils::get_root_node(uri.as_str(), content, "include")?;
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

            parse_results
                .nodes
                .append(&mut ParserUtils::get_all_root_nodes(
                    uri.as_str(),
                    content.as_str(),
                ));

            parse_results.files.push(GitlabFile {
                path: uri.as_str().into(),
                content: content.clone(),
            });

            // arrays are overriden in gitlab.
            let found_stages = ParserUtils::get_stage_definitions(uri.as_str(), content.as_str());
            if !found_stages.is_empty() {
                parse_results.stages = found_stages;
            }

            parse_results
                .variables
                .append(&mut ParserUtils::get_root_variables(
                    uri.as_str(),
                    content.as_str(),
                ));
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
