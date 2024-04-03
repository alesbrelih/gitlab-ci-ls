use log::error;
use lsp_types::Position;
use tree_sitter::{Query, QueryCursor};
use tree_sitter_yaml::language;

use crate::{
    parser::{CompletionType, IncludeInformation, LocalInclude, RemoteInclude},
    GitlabElement, GitlabRootNode, LSPPosition, Range,
};

pub trait Treesitter {
    fn get_root_node(&self, uri: &str, content: &str, node_key: &str) -> Option<GitlabElement>;
    fn get_all_root_nodes(&self, uri: &str, content: &str) -> Vec<GitlabRootNode>;
    fn get_root_variables(&self, uri: &str, content: &str) -> Vec<GitlabElement>;
    fn get_stage_definitions(&self, uri: &str, content: &str) -> Vec<GitlabElement>;
    fn get_all_stages(&self, uri: String, content: &str) -> Vec<GitlabElement>;
    fn get_all_extends(
        &self,
        uri: String,
        content: &str,
        extend_name: Option<&str>,
    ) -> Vec<GitlabElement>;
    fn get_position_type(&self, content: &str, position: Position) -> CompletionType;
}

pub struct TreesitterImpl {}

impl TreesitterImpl {
    pub fn new() -> Self {
        Self {}
    }
}

impl Treesitter for TreesitterImpl {
    fn get_root_node(&self, uri: &str, content: &str, node_key: &str) -> Option<GitlabElement> {
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

        let tree = match parser.parse(content, None) {
            Some(t) => t,
            None => {
                error!("could not parse treesitter Q; got Q:\n{}", query_source);

                return None;
            }
        };

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

    fn get_all_root_nodes(&self, uri: &str, content: &str) -> Vec<GitlabRootNode> {
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

    fn get_root_variables(&self, uri: &str, content: &str) -> Vec<GitlabElement> {
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

    fn get_stage_definitions(&self, uri: &str, content: &str) -> Vec<GitlabElement> {
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

    fn get_all_stages(&self, uri: String, content: &str) -> Vec<GitlabElement> {
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

    fn get_all_extends(
        &self,
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

    fn get_position_type(&self, content: &str, position: Position) -> CompletionType {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(tree_sitter_yaml::language())
            .expect("Error loading YAML grammar");

        // 1. extends
        // 2. stages
        // 3. variables
        // 4. root nodes
        // 4. local includes
        // 5. remote includes
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
            (
                stream(
                    document(
                        block_node(
                            block_mapping(
                                block_mapping_pair
                                    key: (flow_node(plain_scalar(string_scalar)@local_include_key))
                                    value: (
                                        block_node(
                                            block_sequence(
                                                block_sequence_item(
                                                    block_node(
                                                        block_mapping(
                                                            block_mapping_pair
                                                                key: (flow_node(plain_scalar(string_scalar)@local_key))
                                                                value: (flow_node)@local_value
                                                        )
                                                    )
                                                )
                                            )
                                        )
                                    )
                                )
                            )
                        )
                    )
                (#eq? @local_include_key "include")
                (#eq? @local_key "local")
            )
            (
                stream(
                    document(
                        block_node(
                            block_mapping(
                                block_mapping_pair
                                    key: (flow_node(plain_scalar(string_scalar)@remote_include_key))
                                    value: (
                                        block_node(
                                            block_sequence(
                                                block_sequence_item(
                                                    block_node
                                                    [
                                                        (
                                                            block_mapping(
                                                                block_mapping_pair
                                                                    key: (flow_node(plain_scalar(string_scalar)@project_key))
                                                                    value: (flow_node(plain_scalar)@project_value)
                                                            )
                                                        )
                                                        (
                                                            block_mapping(
                                                                block_mapping_pair
                                                                    key: (flow_node(plain_scalar(string_scalar)@ref_key))
                                                                    value: (flow_node(plain_scalar)@ref_value)
                                                            )
                                                        )
                                                        (
                                                            block_mapping(
                                                            block_mapping_pair
                                                                key: (flow_node(plain_scalar(string_scalar)@file_key))
                                                                value: (block_node(block_sequence(block_sequence_item(flow_node)@file)))
                                                            )
                                                        )
                                                    ]
                                                )
                                            )@item
                                        )
                                    )
                                )
                            )
                        )
                    )
                (#eq? @remote_include_key "include")
                (#eq? @ref_key "ref")
                (#eq? @project_key "project")
                (#eq? @file_key "file")
            )
        "#;
        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(language(), query_source).unwrap();
        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut remote_include = RemoteInclude {
            ..Default::default()
        };

        for mat in matches.into_iter() {
            // If this is a remote reference capture, I need to capture multiple values
            // reference,project,file
            // because the way treesitter captures those groups it doesn't really capture all
            // together but there are multiple capture groups I need to iterate over
            // TODO: check treesitter if I can group to make this easier.. Perhaps some capture
            // group is wrong.
            let remote_include_indexes: Vec<u32> = vec![10, 11, 12, 13, 14, 15, 16, 17];
            if mat
                .captures
                .iter()
                .any(|c| remote_include_indexes.contains(&c.index))
            {
                for c in mat.captures {
                    let bounding = match mat.captures.iter().find(|c| c.index == 17) {
                        Some(b) => b,
                        None => {
                            error!("couldn't find index 17 even though its remote capture");

                            return CompletionType::None;
                        }
                    };

                    if bounding.node.start_position().row > position.line as usize
                        && bounding.node.end_position().row < position.line as usize
                    {
                        continue;
                    }

                    match c.index {
                        12 => {
                            remote_include.project = Some(content[c.node.byte_range()].to_string())
                        }
                        14 => {
                            remote_include.reference =
                                Some(content[c.node.byte_range()].to_string())
                        }
                        16 => {
                            if c.node.start_position().row == position.line as usize {
                                remote_include.file =
                                    Some(content[c.node.byte_range()].to_string());
                            }
                        }
                        _ => continue,
                    };
                }

                if remote_include.is_valid() {
                    return CompletionType::Include(IncludeInformation {
                        local: None,
                        remote: Some(remote_include),
                    });
                }
            } else {
                for c in mat.captures {
                    if c.node.start_position().row <= position.line as usize
                        && c.node.end_position().row >= position.line as usize
                    {
                        match c.index {
                            1 => return CompletionType::Extend,
                            3 => return CompletionType::Stage,
                            5 => return CompletionType::Variable,
                            6 => return CompletionType::RootNode,
                            9 => {
                                return CompletionType::Include(IncludeInformation {
                                    local: Some(LocalInclude {
                                        path: content[c.node.byte_range()].to_string(),
                                    }),
                                    remote: None,
                                })
                            }
                            _ => {
                                error!("invalid index: {}", c.index);
                                CompletionType::None
                            }
                        };
                    }
                }
            }
        }

        CompletionType::None
    }
}
