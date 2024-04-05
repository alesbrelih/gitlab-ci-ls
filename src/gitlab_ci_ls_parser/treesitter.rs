use log::error;
use lsp_types::Position;
use tree_sitter::{Query, QueryCursor};
use tree_sitter_yaml::language;

use super::{
    parser, GitlabElement, Include, IncludeInformation, LSPPosition, NodeDefinition, Range,
    RemoteInclude,
};

// TODO: initialize tree only once
pub trait Treesitter {
    fn get_root_node(&self, uri: &str, content: &str, node_key: &str) -> Option<GitlabElement>;
    fn get_all_root_nodes(&self, uri: &str, content: &str) -> Vec<GitlabElement>;
    fn get_root_variables(&self, uri: &str, content: &str) -> Vec<GitlabElement>;
    fn get_stage_definitions(&self, uri: &str, content: &str) -> Vec<GitlabElement>;
    fn get_all_stages(&self, uri: &str, content: &str) -> Vec<GitlabElement>;
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
        needs_name: Option<&str>,
    ) -> Vec<GitlabElement>;
    fn get_position_type(&self, content: &str, position: Position) -> parser::PositionType;
    fn get_root_node_at_position(&self, content: &str, position: Position)
        -> Option<GitlabElement>;
    fn job_variable_definition(
        &self,
        uri: &str,
        content: &str,
        variable_name: &str,
        job_name: &str,
    ) -> Option<GitlabElement>;
}

#[allow(clippy::module_name_repetitions)]
pub struct TreesitterImpl {}

#[allow(clippy::module_name_repetitions)]
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
            (#eq? @key "{node_key}")
        )
        "#
        );

        let Some(tree) = parser.parse(content, None) else {
            error!(
                "could not parse treesitter content; got content:\n{}",
                content
            );

            return None;
        };

        let root_node = tree.root_node();

        let query = match Query::new(language(), query_source.as_str()) {
            Ok(q) => q,
            Err(err) => {
                error!(
                    "could not parse treesitter query; got content:\n{}\ngot error: {}",
                    query_source, err,
                );

                return None;
            }
        };

        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        for mat in matches {
            for c in mat.captures {
                if c.index == 1 {
                    let text = &content[c.node.byte_range()];

                    return Some(GitlabElement {
                        uri: uri.to_string(),
                        key: node_key.to_string(),
                        content: Some(text.to_string()),
                        range: Range {
                            start: LSPPosition {
                                line: u32::try_from(c.node.start_position().row).ok()?,
                                character: u32::try_from(c.node.start_position().column).ok()?,
                            },
                            end: LSPPosition {
                                line: u32::try_from(c.node.end_position().row).ok()?,
                                character: u32::try_from(c.node.end_position().column).ok()?,
                            },
                        },
                    });
                }
            }
        }

        None
    }

    fn get_all_root_nodes(&self, uri: &str, content: &str) -> Vec<GitlabElement> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(tree_sitter_yaml::language())
            .expect("Error loading YAML grammar");

        let query_source = r"
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
        ";

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(language(), query_source).unwrap();

        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut root_nodes = vec![];
        for m in matches {
            let mut node = GitlabElement {
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
                        node.content = Some(text);
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
        for mat in matches {
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
                                line: u32::try_from(c.node.start_position().row).unwrap_or(0),
                                character: u32::try_from(c.node.start_position().column)
                                    .unwrap_or(0),
                            },
                            end: LSPPosition {
                                line: u32::try_from(c.node.end_position().row).unwrap_or(0),
                                character: u32::try_from(c.node.end_position().column).unwrap_or(0),
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
        for mat in matches {
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
                                line: u32::try_from(c.node.start_position().row).unwrap_or(0),
                                character: u32::try_from(c.node.start_position().column)
                                    .unwrap_or(0),
                            },
                            end: LSPPosition {
                                line: u32::try_from(c.node.end_position().row).unwrap_or(0),
                                character: u32::try_from(c.node.end_position().column).unwrap_or(0),
                            },
                        },
                    });
                }
            }
        }

        stages
    }

    fn get_all_stages(&self, uri: &str, content: &str) -> Vec<GitlabElement> {
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
        for mat in matches {
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
                        uri: uri.to_string(),
                        range: Range {
                            start: LSPPosition {
                                line: u32::try_from(c.node.start_position().row).unwrap_or(0),
                                character: u32::try_from(c.node.start_position().column)
                                    .unwrap_or(0),
                            },
                            end: LSPPosition {
                                line: u32::try_from(c.node.end_position().row).unwrap_or(0),
                                character: u32::try_from(c.node.end_position().column).unwrap_or(0),
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

        let mut search = String::new();
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
            {search}
        )
        "#
        );

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(language(), &query_source).unwrap();
        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut extends: Vec<GitlabElement> = vec![];

        for mat in matches {
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
                                line: u32::try_from(c.node.start_position().row).unwrap_or(0),
                                character: u32::try_from(c.node.start_position().column)
                                    .unwrap_or(0),
                            },
                            end: LSPPosition {
                                line: u32::try_from(c.node.end_position().row).unwrap_or(0),
                                character: u32::try_from(c.node.end_position().column).unwrap_or(0),
                            },
                        },
                    });
                }
            }
        }

        extends
    }

    #[allow(clippy::too_many_lines)]
    fn get_position_type(&self, content: &str, position: Position) -> parser::PositionType {
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
            (
                block_mapping_pair
                    key: (flow_node)@needs_key
                    value: (
                    block_node(
                        block_sequence(
                        block_sequence_item(
                            block_node(
                            block_mapping(
                                block_mapping_pair
                                key: (flow_node)@needs_job_key
                                value: (flow_node)@needs_job_value
                            )
                            )
                        )
                        )
                    )
                )
                (#eq? @needs_key "needs")
                (#eq? @needs_job_key "job")
            )
            (
                stream(
                    document(
                        block_node(
                            block_mapping(
                                block_mapping_pair
                                    key: (flow_node(plain_scalar(string_scalar)@remote_url_include_key))
                                    value: (
                                        block_node(
                                            block_sequence(
                                                block_sequence_item(
                                                    block_node(
                                                        block_mapping(
                                                            block_mapping_pair
                                                                key: (flow_node(plain_scalar(string_scalar)@remote_url_key))
                                                                value: (flow_node)@remote_url_value
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
                (#eq? @remote_url_include_key "include")
                (#eq? @remote_url_key "remote")
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

        for mat in matches {
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
                    let Some(bounding) = mat.captures.iter().find(|c| c.index == 17) else {
                        error!("couldn't find index 17 even though its remote capture");

                        return parser::PositionType::None;
                    };

                    if bounding.node.start_position().row > position.line as usize
                        && bounding.node.end_position().row < position.line as usize
                    {
                        continue;
                    }

                    match c.index {
                        12 => {
                            remote_include.project = Some(content[c.node.byte_range()].to_string());
                        }
                        14 => {
                            remote_include.reference =
                                Some(content[c.node.byte_range()].to_string());
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
                    return parser::PositionType::Include(IncludeInformation {
                        remote: Some(remote_include),
                        ..Default::default()
                    });
                }
            } else {
                for c in mat.captures {
                    if c.node.start_position().row <= position.line as usize
                        && c.node.end_position().row >= position.line as usize
                    {
                        match c.index {
                            1 => return parser::PositionType::Extend,
                            3 => return parser::PositionType::Stage,
                            5 => return parser::PositionType::Variable,
                            6 => return parser::PositionType::RootNode,
                            9 => {
                                return parser::PositionType::Include(IncludeInformation {
                                    local: Some(Include {
                                        path: content[c.node.byte_range()].to_string(),
                                    }),
                                    ..Default::default()
                                })
                            }
                            20 => {
                                return parser::PositionType::Needs(NodeDefinition {
                                    name: content[c.node.byte_range()].to_string(),
                                })
                            }
                            23 => {
                                return parser::PositionType::Include(IncludeInformation {
                                    remote_url: Some(Include {
                                        path: content[c.node.byte_range()].to_string(),
                                    }),
                                    ..Default::default()
                                })
                            }
                            _ => {
                                error!("invalid index: {}", c.index);

                                parser::PositionType::None
                            }
                        };
                    }
                }
            }
        }

        parser::PositionType::None
    }

    fn get_all_job_needs(
        &self,
        uri: String,
        content: &str,
        needs_name: Option<&str>,
    ) -> Vec<GitlabElement> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(tree_sitter_yaml::language())
            .expect("Error loading YAML grammar");

        let mut search = String::new();
        if needs_name.is_some() {
            search = format!("(#eq? @needs_job_value \"{}\")", needs_name.unwrap());
        }

        let query_source = format!(
            r#"
            (
                block_mapping_pair
                    key: (flow_node)@needs_key
                    value: (
                    block_node(
                        block_sequence(
                        block_sequence_item(
                            block_node(
                            block_mapping(
                                block_mapping_pair
                                key: (flow_node)@needs_job_key
                                value: (flow_node)@needs_job_value
                            )
                            )
                        )
                        )
                    )
                )
                (#eq? @needs_key "needs")
                (#eq? @needs_job_key "job")
                {search}
            )
        "#
        );

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(language(), &query_source).unwrap();
        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut needs: Vec<GitlabElement> = vec![];

        for mat in matches {
            for c in mat.captures {
                if c.index == 2 {
                    let text = &content[c.node.byte_range()];
                    if c.node.start_position().row != c.node.end_position().row {
                        // sanity check
                        error!(
                            "ALL: extends spans over multiple rows: uri: {} text: {}",
                            uri, text
                        );

                        continue;
                    }

                    needs.push(GitlabElement {
                        key: text.to_owned(),
                        content: None,
                        uri: uri.clone(),
                        range: Range {
                            start: LSPPosition {
                                line: u32::try_from(c.node.start_position().row).unwrap_or(0),
                                character: u32::try_from(c.node.start_position().column)
                                    .unwrap_or(0),
                            },
                            end: LSPPosition {
                                line: u32::try_from(c.node.end_position().row).unwrap_or(0),
                                character: u32::try_from(c.node.end_position().column).unwrap_or(0),
                            },
                        },
                    });
                }
            }
        }

        needs
    }

    fn get_root_node_at_position(
        &self,
        content: &str,
        position: Position,
    ) -> Option<GitlabElement> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(tree_sitter_yaml::language())
            .expect("Error loading YAML grammar");

        let query_source = r"
        (
            stream(
                document(
                    block_node(
                        block_mapping(
                            block_mapping_pair
                                key: (flow_node(plain_scalar(string_scalar)@key))
                        )@full
                    )
                )
            )
        )
        ";

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(language(), query_source).unwrap();

        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        matches
            .into_iter()
            .flat_map(|m| m.captures.iter())
            .find(|c| {
                c.index == 1
                    && c.node.start_position().row <= position.line as usize
                    && c.node.end_position().row >= position.line as usize
            })
            .map(|c| {
                let text = content[c.node.byte_range()].to_string();
                let key = text.lines().collect::<Vec<&str>>()[0]
                    .trim_end_matches(':')
                    .to_string();

                GitlabElement {
                    key,
                    content: Some(text),
                    ..Default::default()
                }
            })
    }

    fn job_variable_definition(
        &self,
        uri: &str,
        content: &str,
        variable_name: &str,
        job_name: &str,
    ) -> Option<GitlabElement> {
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
                                value: (
                                    block_node(
                                        block_mapping(
                                            block_mapping_pair
                                                key: (flow_node(plain_scalar(string_scalar)@property_key))
                                                value: (
                                                    block_node(
                                                        block_mapping(
                                                            block_mapping_pair
                                                            key: (flow_node(plain_scalar(string_scalar)@variable_key))
                                                        )
                                                    )
                                                )
                                            (#eq? @property_key "variables")
                                        )
                                    )
                                )
                            )
                        )
                    )
                )
            (#eq? @key "{job_name}")
            (#eq? @variable_key "{variable_name}")
        )
        "#
        );

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(language(), &query_source).unwrap();

        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        matches
            .into_iter()
            .flat_map(|m| m.captures.iter())
            .find(|c| c.index == 2)
            .map(|c| {
                let text = content[c.node.byte_range()].to_string();
                let key = text.lines().collect::<Vec<&str>>()[0]
                    .trim_end_matches(':')
                    .to_string();

                GitlabElement {
                    uri: uri.to_string(),
                    key,
                    content: Some(text),
                    range: Range {
                        start: LSPPosition {
                            line: u32::try_from(c.node.start_position().row).unwrap_or(0),
                            character: u32::try_from(c.node.start_position().column).unwrap_or(0),
                        },
                        end: LSPPosition {
                            line: u32::try_from(c.node.end_position().row).unwrap_or(0),
                            character: u32::try_from(c.node.end_position().column).unwrap_or(0),
                        },
                    },
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_get_root_node() {
        let cnt = r"
first:
  variables:
    TEST: 5
  list:
    - should
    - be
    - ignored
searched:
  third_var: 3
  forth_var: 4
forth: 5
";

        let key = "searched";
        let uri = "file://mocked";

        let treesitter = TreesitterImpl::new();
        let root_node = treesitter.get_root_node(uri, cnt, "searched");
        assert!(root_node.is_some(), "root_node should be set");

        let root_node = root_node.unwrap();
        assert_eq!(root_node.key, key, "key should be 'searched'");
        assert_eq!(root_node.uri, uri, "uri doesn't match");

        assert!(root_node.content.is_some(), "content should be set");
        let content = root_node.content.unwrap();

        let wanted_content = r"searched:
  third_var: 3
  forth_var: 4";

        assert_eq!(content, wanted_content, "content doesn't match");
        assert_eq!(
            root_node.range.start,
            LSPPosition {
                line: 8,
                character: 0
            },
            "invalid start"
        );

        assert_eq!(
            root_node.range.end,
            LSPPosition {
                line: 10,
                character: 14
            },
            "invalid end"
        );
    }

    #[test]
    fn test_invalid_get_root_node() {
        let cnt = r"
first:
  variables:
    TEST: 5
  list:
    - should
    - be
    - ignored
searched:
  third_var: 3
  forth_var: 4
forth: 5
";

        let uri = "file://mocked";

        let treesitter = TreesitterImpl::new();
        let root_node = treesitter.get_root_node(uri, cnt, "invalid");
        assert!(root_node.is_none(), "root_node should not be set");
    }

    #[test]
    fn test_get_all_root_nodes() {
        let cnt = r"
first:
  variables:
    TEST: 5
  list:
    - should
    - be
    - ignored
searched:
  third_var: 3
  forth_var: 4
forth: 5
";

        let uri = "file://mocked";
        let treesitter = TreesitterImpl::new();
        let root_nodes = treesitter.get_all_root_nodes(uri, cnt);
        assert_eq!(root_nodes.len(), 3, "should find 3 nodes");

        let keys = ["first", "searched", "forth"];

        let cnt_0 = r"first:
  variables:
    TEST: 5
  list:
    - should
    - be
    - ignored";

        let cnt_1 = r"searched:
  third_var: 3
  forth_var: 4";

        let cnt_2 = "forth: 5";

        let cnts = [cnt_0, cnt_1, cnt_2];

        for (idx, node) in root_nodes.iter().enumerate() {
            assert_eq!(node.key, keys[idx]);
            assert_eq!(node.uri, uri);

            assert!(node.content.is_some());
            let content = node.content.clone().unwrap();

            assert_eq!(content, cnts[idx]);
            assert_eq!(
                node.range,
                Range {
                    ..Default::default()
                }
            );
        }
    }

    #[test]
    fn test_get_root_variables() {
        let cnt = r"
first:
  variables:
    TEST: 5
  list:
    - item
variables:
  first_var: 3
  second_var: 4
forth: 5
";

        let uri = "file://mocked";
        let treesitter = TreesitterImpl::new();
        let root_variables = treesitter.get_root_variables(uri, cnt);

        assert_eq!(root_variables.len(), 2);

        let vars = ["first_var", "second_var"];
        let starts = [
            LSPPosition {
                line: 7,
                character: 2,
            },
            LSPPosition {
                line: 8,
                character: 2,
            },
        ];
        let ends = [
            LSPPosition {
                line: 7,
                character: 11,
            },
            LSPPosition {
                line: 8,
                character: 12,
            },
        ];

        for (idx, var) in root_variables.iter().enumerate() {
            assert!(var.content.is_none());
            assert_eq!(var.uri, uri);
            assert_eq!(var.key, vars[idx]);
            assert_eq!(var.key, vars[idx]);
            assert_eq!(var.range.start, starts[idx]);
            assert_eq!(var.range.end, ends[idx]);
        }
    }

    #[test]
    fn test_get_stage_definitions() {
        let cnt = r"
variables:
  first_var: 3
  second_var: 4
stages:
  - first_stage
  - second_stage
";

        let uri = "file://mocked";
        let treesitter = TreesitterImpl::new();
        let stage_definitions = treesitter.get_stage_definitions(uri, cnt);

        assert_eq!(stage_definitions.len(), 2);

        let stages = ["first_stage", "second_stage"];
        let starts = [
            LSPPosition {
                line: 5,
                character: 4,
            },
            LSPPosition {
                line: 6,
                character: 4,
            },
        ];
        let ends = [
            LSPPosition {
                line: 5,
                character: 15,
            },
            LSPPosition {
                line: 6,
                character: 16,
            },
        ];

        for (idx, var) in stage_definitions.iter().enumerate() {
            assert!(var.content.is_none());
            assert_eq!(var.uri, uri);
            assert_eq!(var.key, stages[idx]);
            assert_eq!(var.key, stages[idx]);
            assert_eq!(var.range.start, starts[idx]);
            assert_eq!(var.range.end, ends[idx]);
        }
    }

    #[test]
    fn test_get_all_stages() {
        let cnt = r"
job_one:
  image: alpine
  stage: first

job_two:
  image: ubuntu
  stage: second
";

        let uri = "file://mocked";
        let treesitter = TreesitterImpl::new();
        let all_stages = treesitter.get_all_stages(uri, cnt);

        assert_eq!(all_stages.len(), 2);

        let stages = ["first", "second"];
        let starts = [
            LSPPosition {
                line: 3,
                character: 9,
            },
            LSPPosition {
                line: 7,
                character: 9,
            },
        ];
        let ends = [
            LSPPosition {
                line: 3,
                character: 14,
            },
            LSPPosition {
                line: 7,
                character: 15,
            },
        ];

        for (idx, var) in all_stages.iter().enumerate() {
            assert!(var.content.is_none());
            assert_eq!(var.uri, uri);
            assert_eq!(var.key, stages[idx]);
            assert_eq!(var.key, stages[idx]);
            assert_eq!(var.range.start, starts[idx]);
            assert_eq!(var.range.end, ends[idx]);
        }
    }

    #[test]
    fn test_get_all_extends() {
        let cnt = r"
job_one:
  image: alpine
  extends: .first
  stage: one

job_two:
  image: ubuntu
  extends: .second
  stage: two
";

        let uri = "file://mocked";
        let treesitter = TreesitterImpl::new();
        let all_extends = treesitter.get_all_extends(uri.to_string(), cnt, None);

        assert_eq!(all_extends.len(), 2);

        let extends = [".first", ".second"];
        let starts = [
            LSPPosition {
                line: 3,
                character: 11,
            },
            LSPPosition {
                line: 8,
                character: 11,
            },
        ];
        let ends = [
            LSPPosition {
                line: 3,
                character: 17,
            },
            LSPPosition {
                line: 8,
                character: 18,
            },
        ];

        for (idx, var) in all_extends.iter().enumerate() {
            assert!(var.content.is_none());
            assert_eq!(var.uri, uri);
            assert_eq!(var.key, extends[idx]);
            assert_eq!(var.key, extends[idx]);
            assert_eq!(var.range.start, starts[idx]);
            assert_eq!(var.range.end, ends[idx]);
        }
    }

    #[test]
    fn test_get_all_extends_with_name() {
        let cnt = r"
job_one:
  image: alpine
  extends: .first
  stage: one

job_two:
  image: ubuntu
  extends: .second
  stage: two
";

        let uri = "file://mocked";
        let treesitter = TreesitterImpl::new();
        let all_extends = treesitter.get_all_extends(uri.to_string(), cnt, Some(".second"));

        assert_eq!(all_extends.len(), 1);

        let extends = [".second"];
        let starts = [LSPPosition {
            line: 8,
            character: 11,
        }];
        let ends = [LSPPosition {
            line: 8,
            character: 18,
        }];

        for (idx, var) in all_extends.iter().enumerate() {
            assert!(var.content.is_none());
            assert_eq!(var.uri, uri);
            assert_eq!(var.key, extends[idx]);
            assert_eq!(var.key, extends[idx]);
            assert_eq!(var.range.start, starts[idx]);
            assert_eq!(var.range.end, ends[idx]);
        }
    }

    #[test]
    fn test_get_all_extends_no_results() {
        let cnt = r"
job_one:
  image: alpine
  extends: .first
  stage: one

job_two:
  image: ubuntu
  extends: .second
  stage: two
";

        let uri = "file://mocked";
        let treesitter = TreesitterImpl::new();
        let all_extends = treesitter.get_all_extends(uri.to_string(), cnt, Some(".invalid"));

        assert_eq!(all_extends.len(), 0);
    }
}
