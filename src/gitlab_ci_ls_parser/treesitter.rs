use log::error;
use lsp_types::Position;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor};

use super::{
    parser, parser_utils::ParserUtils, treesitter_queries::TreesitterQueries, Component,
    ComponentInput, ComponentInputValueBlock, ComponentInputValuePlain, GitlabCacheElement,
    GitlabComponentElement, GitlabElement, GitlabInputElement, Include, IncludeInformation,
    LSPPosition, NodeDefinition, Range, RemoteInclude, RuleReference,
};
use mockall::{automock, predicate::str};

// TODO: initialize tree only once

#[allow(clippy::ref_option_ref)]
#[cfg_attr(test, automock)]
pub trait Treesitter {
    fn get_root_node(&self, uri: &str, content: &str, node_key: &str) -> Option<GitlabElement>;
    fn get_root_node_key(&self, uri: &str, content: &str, node_key: &str) -> Option<GitlabElement>;
    fn get_all_root_nodes(&self, uri: &str, content: &str) -> Vec<GitlabElement>;
    fn get_root_variables(&self, uri: &str, content: &str) -> Vec<GitlabElement>;
    fn get_stage_definitions(&self, uri: &str, content: &str) -> Vec<GitlabElement>;
    fn get_all_components(&self, uri: &str, content: &str) -> Vec<GitlabComponentElement>;
    fn get_all_multi_caches(&self, uri: &str, content: &str) -> Vec<GitlabCacheElement>;
    fn get_all_stages<'a>(
        &self,
        uri: &'a str,
        content: &'a str,
        stage: Option<&'a str>,
    ) -> Vec<GitlabElement>;
    fn get_all_rule_references<'a>(
        &self,
        uri: &'a str,
        content: &'a str,
        rule: Option<&'a str>,
    ) -> Vec<GitlabElement>;
    fn get_all_extends<'a>(
        &self,
        uri: String,
        content: &'a str,
        extend_name: Option<&'a str>,
    ) -> Vec<GitlabElement>;
    fn get_all_job_needs<'a>(
        &self,
        uri: String,
        content: &'a str,
        needs_name: Option<&'a str>,
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
    fn get_component_spec_inputs(&self, content: &str) -> Option<String>;
}

#[allow(clippy::module_name_repetitions)]
pub struct TreesitterImpl {}

#[allow(clippy::module_name_repetitions)]
impl TreesitterImpl {
    pub fn new() -> Self {
        Self {}
    }

    #[allow(clippy::too_many_arguments)]
    fn get_position_type_component(
        mat: &tree_sitter::QueryMatch<'_, '_>,
        position: Position,
        content: &str,
        full_component_index: u32,
        component_uri_index: u32,
        component_input_index: u32,
        component_input_value_plain_index: u32,
        component_input_value_block_index: u32,
        component_input_error: u32,
    ) -> Option<parser::PositionType> {
        let mut component = Component {
            ..Default::default()
        };

        let Some(full) = mat
            .captures
            .iter()
            .find(|c| c.index == full_component_index)
        else {
            error!("couldn't find index {full_component_index} even though its component capture");

            return None;
        };

        if full.node.start_position().row <= position.line as usize
            && full.node.end_position().row >= position.line as usize
        {
            let mut inputs = vec![];
            let mut input = None;
            for c in mat.captures {
                match c.index {
                    idx if idx == component_uri_index => {
                        let value = content[c.node.byte_range()].to_string();
                        component.uri = ParserUtils::strip_quotes(&value).to_string();
                    }
                    idx if idx == component_input_index => {
                        if let Some(i) = input {
                            inputs.push(i);
                        }

                        let key = content[c.node.byte_range()].to_string();
                        let hovered = c.node.start_position().row == position.line as usize
                            && position.character as usize >= c.node.start_position().column
                            && position.character as usize <= c.node.end_position().column;

                        input = Some(ComponentInput {
                            key: ParserUtils::strip_quotes(&key).to_string(),
                            hovered,
                            ..Default::default()
                        });
                    }
                    idx if idx == component_input_error => {
                        let key = content[c.node.byte_range()].to_string();
                        let hovered = c.node.start_position().row == position.line as usize
                            && position.character as usize >= c.node.start_position().column
                            && position.character as usize <= c.node.end_position().column;

                        inputs.push(ComponentInput {
                            key: ParserUtils::strip_quotes(&key).to_string(),
                            hovered,
                            ..Default::default()
                        });
                    }
                    idx if idx == component_input_value_plain_index => {
                        if let Some(ref mut i) = input {
                            let hovered = c.node.start_position().row == position.line as usize
                                && position.character as usize >= c.node.start_position().column
                                && position.character as usize <= c.node.end_position().column;

                            let value = content[c.node.byte_range()].to_string();
                            i.value_plain = ComponentInputValuePlain {
                                value: ParserUtils::strip_quotes(&value).to_string(),
                                hovered,
                            }
                        }
                    }
                    idx if idx == component_input_value_block_index => {
                        if let Some(ref mut i) = input {
                            let hovered = c.node.start_position().row == position.line as usize
                                && position.character as usize >= c.node.start_position().column
                                && position.character as usize <= c.node.end_position().column;
                            let value = content[c.node.byte_range()].to_string();

                            i.value_block = ComponentInputValueBlock {
                                value: ParserUtils::strip_quotes(&value).to_string(),
                                hovered,
                            }
                        }
                    }
                    _ => {}
                }
            }

            if let Some(i) = input {
                inputs.push(i);
            }

            component.inputs = inputs;

            return Some(parser::PositionType::Include(IncludeInformation {
                remote: None,
                remote_url: None,
                local: None,
                basic: None,
                component: Some(component),
            }));
        }

        None
    }
}

impl Treesitter for TreesitterImpl {
    fn get_root_node(&self, uri: &str, content: &str, node_key: &str) -> Option<GitlabElement> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = match Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_root_node(node_key),
        ) {
            Ok(q) => q,
            Err(err) => {
                error!(
                    "could not parse treesitter query; got content:\n{}\ngot error: {}",
                    &TreesitterQueries::get_root_node(node_key),
                    err,
                );

                return None;
            }
        };

        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        while let Some(mat) = matches.next() {
            for c in mat.captures {
                if c.index == 1 {
                    let text = &content[c.node.byte_range()];

                    return Some(GitlabElement {
                        uri: uri.to_string(),
                        key: ParserUtils::strip_quotes(node_key).to_string(),
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
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_all_root_nodes(),
        )
        .unwrap();

        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut root_nodes = vec![];
        while let Some(m) = matches.next() {
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
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        // TODO: this should be generic fn accepting treesitter query

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_root_variables(),
        )
        .unwrap();
        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut environments = vec![];
        while let Some(mat) = matches.next() {
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
                        key: ParserUtils::strip_quotes(text).to_string(),
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
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_stage_definitions(),
        )
        .unwrap();
        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut stages = vec![];
        while let Some(mat) = matches.next() {
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
                        key: ParserUtils::strip_quotes(text).to_string(),
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

    fn get_all_stages(&self, uri: &str, content: &str, stage: Option<&str>) -> Vec<GitlabElement> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_all_stages(stage),
        )
        .unwrap();
        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut extends: Vec<GitlabElement> = vec![];

        let valid_indexes: Vec<u32> = vec![1, 2];
        while let Some(mat) = matches.next() {
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
                        key: ParserUtils::strip_quotes(text).to_string(),
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
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_all_extends(extend_name),
        )
        .unwrap();
        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut extends: Vec<GitlabElement> = vec![];

        while let Some(mat) = matches.next() {
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
                        key: ParserUtils::strip_quotes(text).to_string(),
                        content: None,
                        uri: uri.clone(),
                        range: get_range(c.node, text).unwrap_or_default(),
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
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_position_type(),
        )
        .unwrap();
        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut remote_include = RemoteInclude {
            ..Default::default()
        };

        let extends_index = query.capture_index_for_name("extends").unwrap();
        let stage_index = query.capture_index_for_name("stage").unwrap();
        let variable_index = query.capture_index_for_name("variable").unwrap();
        let root_node_index = query.capture_index_for_name("rootnode").unwrap();
        let local_include_index = query.capture_index_for_name("local_value").unwrap();
        let needs_index = query.capture_index_for_name("needs_job_value").unwrap();
        let remote_url_index = query.capture_index_for_name("remote_url_value").unwrap();
        let project_name_index = query.capture_index_for_name("project_value").unwrap();
        let project_ref_index = query.capture_index_for_name("ref_value").unwrap();
        let project_file_index = query.capture_index_for_name("file_value").unwrap();
        let project_item_index = query.capture_index_for_name("remote_include_item").unwrap();
        let basic_include_index = query.capture_index_for_name("basic_include_value").unwrap();
        let rule_reference_index = query
            .capture_index_for_name("rule_reference_value")
            .unwrap();
        let component_uri_index = query.capture_index_for_name("component_uri").unwrap();
        let component_input_index = query.capture_index_for_name("component_input").unwrap();
        let component_input_error_index = query
            .capture_index_for_name("component_input_error")
            .unwrap();
        let component_input_value_plain_index = query
            .capture_index_for_name("component_input_value_plain")
            .unwrap();
        let component_input_value_block_index = query
            .capture_index_for_name("component_input_value_block")
            .unwrap();
        let full_component_index = query.capture_index_for_name("full_component").unwrap();
        let dependency_index = query.capture_index_for_name("dependency").unwrap();

        while let Some(mat) = matches.next() {
            // If this is a remote reference capture, I need to capture multiple values
            // reference,project,file
            // because the way treesitter captures those groups it doesn't really capture all
            // together but there are multiple capture groups I need to iterate over
            // TODO: check treesitter if I can group to make this easier.. Perhaps some capture
            // group is wrong.
            let remote_include_indexes =
                [project_name_index, project_ref_index, project_file_index];

            // If its component
            if mat.captures.iter().any(|c| c.index == full_component_index) {
                if let Some(position_type) = TreesitterImpl::get_position_type_component(
                    mat,
                    position,
                    content,
                    full_component_index,
                    component_uri_index,
                    component_input_index,
                    component_input_value_plain_index,
                    component_input_value_block_index,
                    component_input_error_index,
                ) {
                    return position_type;
                }
            } else if mat
                .captures
                .iter()
                .any(|c| remote_include_indexes.contains(&c.index))
            {
                let Some(bounding) = mat.captures.iter().find(|c| c.index == project_item_index)
                else {
                    error!(
                        "couldn't find index {project_item_index} even though its remote capture"
                    );

                    return parser::PositionType::None;
                };

                if bounding.node.start_position().row > position.line as usize
                    && bounding.node.end_position().row < position.line as usize
                {
                    continue;
                }
                for c in mat.captures {
                    match c.index {
                        idx if idx == project_name_index => {
                            remote_include.project = Some(
                                ParserUtils::strip_quotes(&content[c.node.byte_range()])
                                    .to_string(),
                            );
                        }
                        idx if idx == project_ref_index => {
                            remote_include.reference = Some(
                                ParserUtils::strip_quotes(&content[c.node.byte_range()])
                                    .to_string(),
                            );
                        }
                        idx if idx == project_file_index => {
                            if c.node.start_position().row == position.line as usize {
                                remote_include.file =
                                    Some(content[c.node.byte_range()].to_string());
                            }
                        }
                        _ => {}
                    }
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
                        && c.node.start_position().column <= position.character as usize
                        && c.node.end_position().column >= position.character as usize
                    {
                        match c.index {
                            idx if idx == extends_index => return parser::PositionType::Extend,
                            idx if idx == stage_index => return parser::PositionType::Stage,
                            idx if idx == dependency_index => {
                                return parser::PositionType::Dependency
                            }
                            idx if idx == variable_index => return parser::PositionType::Variable,
                            idx if idx == root_node_index => return parser::PositionType::RootNode,
                            idx if idx == local_include_index => {
                                return parser::PositionType::Include(IncludeInformation {
                                    local: Some(Include {
                                        path: content[c.node.byte_range()].to_string(),
                                    }),
                                    ..Default::default()
                                })
                            }
                            idx if idx == needs_index => {
                                return parser::PositionType::Needs(NodeDefinition {
                                    name: content[c.node.byte_range()].to_string(),
                                })
                            }
                            idx if idx == remote_url_index => {
                                return parser::PositionType::Include(IncludeInformation {
                                    remote_url: Some(Include {
                                        path: content[c.node.byte_range()].to_string(),
                                    }),
                                    ..Default::default()
                                })
                            }
                            idx if idx == basic_include_index => {
                                return parser::PositionType::Include(IncludeInformation {
                                    basic: Some(Include {
                                        path: content[c.node.byte_range()].to_string(),
                                    }),
                                    ..Default::default()
                                })
                            }
                            idx if idx == rule_reference_index => {
                                return parser::PositionType::RuleReference(RuleReference {
                                    node: content[c.node.byte_range()]
                                        .trim_matches('\'')
                                        .trim_matches('"')
                                        .to_string(),
                                })
                            }
                            _ => {
                                error!("invalid index: {}", c.index);
                                error!(
                                    "invalid index content: {}",
                                    content[c.node.byte_range()].to_string()
                                );

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
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_all_job_needs(needs_name),
        )
        .unwrap();
        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut needs: Vec<GitlabElement> = vec![];

        while let Some(mat) = matches.next() {
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
                        key: ParserUtils::strip_quotes(text).to_string(),
                        content: None,
                        uri: uri.clone(),
                        range: get_range(c.node, text).unwrap_or_default(),
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
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_root_node_at_position(),
        )
        .unwrap();

        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        while let Some(m) = matches.next() {
            // Iterate through the captures for this match
            for capture in m.captures {
                if capture.index == 1
                    && capture.node.start_position().row <= position.line as usize
                    && capture.node.end_position().row >= position.line as usize
                {
                    // Extract the text and create the GitlabElement
                    let text = content[capture.node.byte_range()].to_string();
                    let key = text.lines().collect::<Vec<&str>>()[0]
                        .trim_end_matches(':')
                        .to_string();

                    return Some(GitlabElement {
                        key,
                        content: Some(text),
                        ..Default::default()
                    });
                }
            }
        }

        None
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
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_job_variable_definition(job_name, variable_name),
        )
        .unwrap();

        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        while let Some(m) = matches.next() {
            // Iterate over the captures for this match
            for capture in m.captures {
                if capture.index == 2 {
                    // Found the match, now create and return the GitlabElement
                    return Some(GitlabElement {
                        uri: uri.to_string(),
                        key: ParserUtils::strip_quotes(&content[capture.node.byte_range()])
                            .to_string(),
                        content: None,
                        range: Range {
                            start: LSPPosition {
                                line: u32::try_from(capture.node.start_position().row).unwrap_or(0),
                                character: u32::try_from(capture.node.start_position().column)
                                    .unwrap_or(0),
                            },
                            end: LSPPosition {
                                line: u32::try_from(capture.node.end_position().row).unwrap_or(0),
                                character: u32::try_from(capture.node.end_position().column)
                                    .unwrap_or(0),
                            },
                        },
                    });
                }
            }
        }

        None
    }

    fn get_component_spec_inputs(&self, content: &str) -> Option<String> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_component_spec_inputs(),
        )
        .unwrap();

        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());
        let spec_index = query.capture_index_for_name("spec").unwrap();

        while let Some(mat) = matches.next() {
            for c in mat.captures {
                if c.index == spec_index {
                    return Some(content[c.node.byte_range()].to_string());
                }
            }
        }

        None
    }

    #[allow(clippy::too_many_lines)]
    fn get_all_components(&self, uri: &str, content: &str) -> Vec<GitlabComponentElement> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_all_components(),
        )
        .unwrap();

        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let component_uri_index = query.capture_index_for_name("component_uri").unwrap();
        let component_input_index = query.capture_index_for_name("component_input").unwrap();

        // number/string
        let component_input_value_plain_index = query
            .capture_index_for_name("component_input_value_plain")
            .unwrap();

        let component_input_value_block_index = query
            .capture_index_for_name("component_input_value_block")
            .unwrap();

        let full_component_index = query.capture_index_for_name("full_component").unwrap();

        let mut components = vec![];
        while let Some(m) = matches.next() {
            let mut node = GitlabComponentElement {
                uri: uri.to_string(),
                ..Default::default()
            };
            let mut current_input = None;
            for c in m.captures {
                let text = content[c.node.byte_range()].to_string();
                match c.index {
                    idx if idx == component_uri_index => {
                        node.key = text;
                    }
                    idx if idx == full_component_index => {
                        node.content = Some(text);
                        node.range = Range {
                            start: LSPPosition {
                                line: u32::try_from(c.node.start_position().row).unwrap_or(0),
                                character: u32::try_from(c.node.start_position().column)
                                    .unwrap_or(0),
                            },
                            end: LSPPosition {
                                line: u32::try_from(c.node.end_position().row).unwrap_or(0),
                                character: u32::try_from(c.node.end_position().column).unwrap_or(0),
                            },
                        };
                    }
                    idx if idx == component_input_index => {
                        if let Some(current) = current_input {
                            node.inputs.push(current);
                        }

                        current_input = Some(GitlabInputElement {
                            uri: uri.to_string(),
                            content: Some(text.clone()),
                            key: ParserUtils::strip_quotes(&text).to_string(),
                            range: Range {
                                start: LSPPosition {
                                    line: u32::try_from(c.node.start_position().row).unwrap_or(0),
                                    character: u32::try_from(c.node.start_position().column)
                                        .unwrap_or(0),
                                },
                                end: LSPPosition {
                                    line: u32::try_from(c.node.end_position().row).unwrap_or(0),
                                    character: u32::try_from(c.node.end_position().column)
                                        .unwrap_or(0),
                                },
                            },
                            value_plain: None,
                            value_block: None,
                        });
                    }
                    idx if idx == component_input_value_plain_index => {
                        if let Some(ref mut current) = current_input {
                            current.value_plain = Some(GitlabElement {
                                uri: uri.to_string(),
                                content: Some(text.clone()),
                                key: ParserUtils::strip_quotes(&text).to_string(),
                                range: Range {
                                    start: LSPPosition {
                                        line: u32::try_from(c.node.start_position().row)
                                            .unwrap_or(0),
                                        character: u32::try_from(c.node.start_position().column)
                                            .unwrap_or(0),
                                    },
                                    end: LSPPosition {
                                        line: u32::try_from(c.node.end_position().row).unwrap_or(0),
                                        character: u32::try_from(c.node.end_position().column)
                                            .unwrap_or(0),
                                    },
                                },
                            });
                        }
                    }
                    idx if idx == component_input_value_block_index => {
                        if let Some(ref mut current) = current_input {
                            current.value_block = Some(GitlabElement {
                                uri: uri.to_string(),
                                content: Some(text.clone()),
                                key: ParserUtils::strip_quotes(&text).to_string(),
                                range: Range {
                                    start: LSPPosition {
                                        line: u32::try_from(c.node.start_position().row)
                                            .unwrap_or(0),
                                        character: u32::try_from(c.node.start_position().column)
                                            .unwrap_or(0),
                                    },
                                    end: LSPPosition {
                                        line: u32::try_from(c.node.end_position().row).unwrap_or(0),
                                        character: u32::try_from(c.node.end_position().column)
                                            .unwrap_or(0),
                                    },
                                },
                            });
                        }
                    }
                    _ => {}
                }
            }

            if let Some(current) = current_input {
                node.inputs.push(current);
            }

            components.push(node);
        }

        components
    }

    fn get_all_rule_references(
        &self,
        uri: &str,
        content: &str,
        rule: Option<&str>,
    ) -> Vec<GitlabElement> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_all_rule_references(rule),
        )
        .unwrap();

        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut extends: Vec<GitlabElement> = vec![];

        while let Some(mat) = matches.next() {
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

                    extends.push(GitlabElement {
                        key: ParserUtils::strip_quotes(text).to_string(),
                        content: None,
                        uri: uri.to_string(),
                        range: get_range(c.node, text).unwrap_or_default(),
                    });
                }
            }
        }

        extends
    }

    fn get_root_node_key(&self, uri: &str, content: &str, node_key: &str) -> Option<GitlabElement> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = match Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_root_node_key(node_key),
        ) {
            Ok(q) => q,
            Err(err) => {
                error!(
                    "could not parse treesitter query; got content:\n{}\ngot error: {}",
                    &TreesitterQueries::get_root_node(node_key),
                    err,
                );

                return None;
            }
        };

        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        while let Some(mat) = matches.next() {
            for c in mat.captures {
                if c.index == 0 {
                    let text = &content[c.node.byte_range()];

                    return Some(GitlabElement {
                        uri: uri.to_string(),
                        key: ParserUtils::strip_quotes(node_key).to_string(),
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

    fn get_all_multi_caches(&self, uri: &str, content: &str) -> Vec<GitlabCacheElement> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .expect("Error loading YAML grammar");

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(
            &tree_sitter_yaml::LANGUAGE.into(),
            &TreesitterQueries::get_all_caches(),
        )
        .unwrap();

        let mut cursor_qry = QueryCursor::new();
        let mut matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let cache_node_index = query.capture_index_for_name("cache_node").unwrap();
        let cache_item = query.capture_index_for_name("cache_item").unwrap();

        let mut components = vec![];
        while let Some(m) = matches.next() {
            let mut node = GitlabCacheElement {
                key: "cache".to_string(),
                uri: uri.to_string(),
                ..Default::default()
            };

            for c in m.captures {
                let text = content[c.node.byte_range()].to_string();
                match c.index {
                    idx if idx == cache_node_index => {
                        node.content = Some(text);
                        node.range = Range {
                            start: LSPPosition {
                                line: u32::try_from(c.node.start_position().row).unwrap_or(0),
                                character: u32::try_from(c.node.start_position().column)
                                    .unwrap_or(0),
                            },
                            end: LSPPosition {
                                line: u32::try_from(c.node.end_position().row).unwrap_or(0),
                                character: u32::try_from(c.node.end_position().column).unwrap_or(0),
                            },
                        };
                    }
                    idx if idx == cache_item => {
                        let cache_item = GitlabElement {
                            uri: uri.to_string(),
                            content: Some(text.clone()),
                            key: ParserUtils::strip_quotes(&text).to_string(),
                            range: Range {
                                start: LSPPosition {
                                    line: u32::try_from(c.node.start_position().row).unwrap_or(0),
                                    character: u32::try_from(c.node.start_position().column)
                                        .unwrap_or(0),
                                },
                                end: LSPPosition {
                                    line: u32::try_from(c.node.end_position().row).unwrap_or(0),
                                    character: u32::try_from(c.node.end_position().column)
                                        .unwrap_or(0),
                                },
                            },
                        };

                        node.cache_items.push(cache_item);
                    }
                    _ => {}
                }
            }

            components.push(node);
        }

        components
    }
}

fn get_range(node: Node<'_>, text: &str) -> anyhow::Result<Range> {
    let mut start_character = u32::try_from(node.start_position().column)?;
    let mut end_character = u32::try_from(node.end_position().column)?;
    if text.starts_with('\'') || text.starts_with('"') {
        start_character += 1;
        end_character -= 1;
    }

    Ok(Range {
        start: LSPPosition {
            line: u32::try_from(node.start_position().row)?,
            character: start_character,
        },
        end: LSPPosition {
            line: u32::try_from(node.end_position().row)?,
            character: end_character,
        },
    })
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
        let all_stages = treesitter.get_all_stages(uri, cnt, None);

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

        for (idx, extend) in all_extends.iter().enumerate() {
            assert!(extend.content.is_none());
            assert_eq!(extend.uri, uri);
            assert_eq!(extend.key, extends[idx]);
            assert_eq!(extend.key, extends[idx]);
            assert_eq!(extend.range.start, starts[idx]);
            assert_eq!(extend.range.end, ends[idx]);
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

        for (idx, extend) in all_extends.iter().enumerate() {
            assert!(extend.content.is_none());
            assert_eq!(extend.uri, uri);
            assert_eq!(extend.key, extends[idx]);
            assert_eq!(extend.key, extends[idx]);
            assert_eq!(extend.range.start, starts[idx]);
            assert_eq!(extend.range.end, ends[idx]);
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

    #[test]
    fn test_get_all_job_needs() {
        let cnt = r"
job_one:
  image: alpine
  extends: .first
  stage: one
  needs:
    - job: job_one

job_two:
  image: ubuntu
  extends: .second
  stage: two
  needs:
    - job: job_two_len
";

        let uri = "file://mocked";
        let treesitter = TreesitterImpl::new();
        let all_job_needs = treesitter.get_all_job_needs(uri.to_string(), cnt, None);

        assert_eq!(all_job_needs.len(), 2);

        let extends = ["job_one", "job_two_len"];
        let starts = [
            LSPPosition {
                line: 6,
                character: 11,
            },
            LSPPosition {
                line: 13,
                character: 11,
            },
        ];
        let ends = [
            LSPPosition {
                line: 6,
                character: 18,
            },
            LSPPosition {
                line: 13,
                character: 22,
            },
        ];

        for (idx, need) in all_job_needs.iter().enumerate() {
            assert!(need.content.is_none());
            assert_eq!(need.uri, uri);
            assert_eq!(need.key, extends[idx]);
            assert_eq!(need.key, extends[idx]);
            assert_eq!(need.range.start, starts[idx]);
            assert_eq!(need.range.end, ends[idx]);
        }
    }

    #[test]
    fn test_get_all_job_needs_with_name() {
        let cnt = r"
job_one:
  image: alpine
  extends: .first
  stage: one
  needs:
    - job: job_one

job_two:
  image: ubuntu
  extends: .second
  stage: two
  needs:
    - job: job_two_len
";

        let uri = "file://mocked";
        let treesitter = TreesitterImpl::new();
        let all_job_needs = treesitter.get_all_job_needs(uri.to_string(), cnt, Some("job_two_len"));

        let extends = ["job_two_len"];
        assert_eq!(all_job_needs.len(), extends.len());

        let starts = [LSPPosition {
            line: 13,
            character: 11,
        }];
        let ends = [LSPPosition {
            line: 13,
            character: 22,
        }];

        for (idx, need) in all_job_needs.iter().enumerate() {
            assert!(need.content.is_none());
            assert_eq!(need.uri, uri);
            assert_eq!(need.key, extends[idx]);
            assert_eq!(need.key, extends[idx]);
            assert_eq!(need.range.start, starts[idx]);
            assert_eq!(need.range.end, ends[idx]);
        }
    }

    #[test]
    fn test_get_all_job_needs_with_invalid_name() {
        let cnt = r"
job_one:
  image: alpine
  extends: .first
  stage: one
  needs:
    - job: job_one

job_two:
  image: ubuntu
  extends: .second
  stage: two
  needs:
    - job: job_two_len
";

        let uri = "file://mocked";
        let treesitter = TreesitterImpl::new();
        let all_job_needs = treesitter.get_all_job_needs(uri.to_string(), cnt, Some("invalid"));

        assert_eq!(all_job_needs.len(), 0);
    }

    #[test]
    fn test_get_root_node_at_position() {
        let cnt = r"
job_one:
  image: alpine
  extends: .first
  stage: one
  needs:
    - job: job_one

job_two:
  image: ubuntu
  extends: .second
  stage: two
  needs:
    - job: job_two_len
";

        let treesitter = TreesitterImpl::new();
        let root_node = treesitter.get_root_node_at_position(
            cnt,
            Position {
                line: 9,
                character: 10,
            },
        );

        let wanted_cnt = r"job_two:
  image: ubuntu
  extends: .second
  stage: two
  needs:
    - job: job_two_len
";

        assert!(root_node.is_some());
        let root_node = root_node.unwrap();
        assert_eq!(root_node.key, "job_two");
        assert_eq!(root_node.content, Some(wanted_cnt.to_string()));
    }

    #[test]
    fn test_get_root_node_at_position_invalid_position() {
        let cnt = r"
job_one:
  image: alpine
  extends: .first
  stage: one
  needs:
    - job: job_one

job_two:
  image: ubuntu
  extends: .second
  stage: two
  needs:
    - job: job_two_len
";

        let treesitter = TreesitterImpl::new();
        let root_node = treesitter.get_root_node_at_position(
            cnt,
            Position {
                line: 20,
                character: 10,
            },
        );

        assert!(root_node.is_none());
    }

    #[test]
    fn test_job_variable_definition() {
        let cnt = r"
job_one:
  image: alpine
  extends: .first
  stage: one
  variables:
    SEARCHED: no
    OTHER: yes
  needs:
    - job: job_one
";

        let uri = "file://mocked";
        let treesitter = TreesitterImpl::new();
        let variable_definition =
            treesitter.job_variable_definition(uri, cnt, "SEARCHED", "job_one");

        assert!(variable_definition.is_some());

        let variable_definition = variable_definition.unwrap();
        assert_eq!(variable_definition.key, "SEARCHED");
        assert!(variable_definition.content.is_none());
        assert_eq!(
            variable_definition.range.start,
            LSPPosition {
                line: 6,
                character: 4,
            }
        );
        assert_eq!(
            variable_definition.range.end,
            LSPPosition {
                line: 6,
                character: 12,
            }
        );
    }

    #[test]
    fn test_job_variable_definition_invalid_job_name() {
        let cnt = r"
job_one:
  image: alpine
  extends: .first
  stage: one
  variables:
    SEARCHED: no
    OTHER: yes
  needs:
    - job: job_one
";

        let uri = "file://mocked";
        let treesitter = TreesitterImpl::new();
        let variable_definition =
            treesitter.job_variable_definition(uri, cnt, "SEARCHED", "invalid_job");

        assert!(variable_definition.is_none());
    }

    #[test]
    fn test_job_variable_definition_invalid_var_name() {
        let cnt = r"
job_one:
  image: alpine
  extends: .first
  stage: one
  variables:
    SEARCHED: no
    OTHER: yes
  needs:
    - job: job_one
";

        let uri = "file://mocked";
        let treesitter = TreesitterImpl::new();
        let variable_definition = treesitter.job_variable_definition(uri, cnt, "NOVAR", "job_one");

        assert!(variable_definition.is_none());
    }

    #[test]
    fn test_get_position_type_project() {
        let cnt = r#"
include:
  - project: myproject/name
    ref: 1.5.0
    file:
      - "/resources/ci-templates/mytemplate.yml"
  - local: ".my-local.yml"
  - remote: "https://myremote.com/template.yml"

job_one:
  image: alpine
  extends: .first
  stage: one
  variables:
    SEARCHED: no
    OTHER: yes
  needs:
    - job: job_one
"#;

        let treesitter = TreesitterImpl::new();
        let project_file = treesitter.get_position_type(
            cnt,
            Position {
                line: 5,
                character: 13,
            },
        );

        let want_project = "myproject/name";
        let want_reference = "1.5.0".to_string();
        let want_file = "\"/resources/ci-templates/mytemplate.yml\"".to_string();
        match project_file {
            parser::PositionType::Include(IncludeInformation {
                remote:
                    Some(RemoteInclude {
                        project: Some(project),
                        reference: Some(reference),
                        file: Some(file),
                    }),
                local: None,
                remote_url: None,
                basic: None,
                component: None,
            }) => {
                assert_eq!(want_project, project);
                assert_eq!(want_reference, reference);
                assert_eq!(want_file, file);
            }
            _ => panic!("project file is invalid"),
        }
    }

    #[test]
    fn test_get_position_type_project_no_ref() {
        let cnt = r#"
include:
  - project: myproject/name
    file:
      - "/resources/ci-templates/mytemplate.yml"
  - local: ".my-local.yml"
  - remote: "https://myremote.com/template.yml"

job_one:
  image: alpine
  extends: .first
  stage: one
  variables:
    SEARCHED: no
    OTHER: yes
  needs:
    - job: job_one
"#;

        let treesitter = TreesitterImpl::new();
        let project_file = treesitter.get_position_type(
            cnt,
            Position {
                line: 4,
                character: 13,
            },
        );

        let want_project = "myproject/name";
        let want_file = "\"/resources/ci-templates/mytemplate.yml\"".to_string();
        match project_file {
            parser::PositionType::Include(IncludeInformation {
                remote:
                    Some(RemoteInclude {
                        project: Some(project),
                        reference,
                        file: Some(file),
                    }),
                local: None,
                remote_url: None,
                basic: None,
                component: None,
            }) => {
                assert_eq!(want_project, project);
                assert_eq!(None, reference);
                assert_eq!(want_file, file);
            }
            _ => panic!("project file is invalid"),
        }
    }

    #[test]
    fn test_get_position_type_project_no_ref_single_file() {
        let cnt = r#"
include:
  - project: myproject/name
    file: "/resources/ci-templates/mytemplate.yml"
  - local: ".my-local.yml"
  - remote: "https://myremote.com/template.yml"

job_one:
  image: alpine
  extends: .first
  stage: one
  variables:
    SEARCHED: no
    OTHER: yes
  needs:
    - job: job_one
"#;

        let treesitter = TreesitterImpl::new();
        let project_file = treesitter.get_position_type(
            cnt,
            Position {
                line: 3,
                character: 13,
            },
        );

        let want_project = "myproject/name";
        let want_file = "\"/resources/ci-templates/mytemplate.yml\"".to_string();
        match project_file {
            parser::PositionType::Include(IncludeInformation {
                remote:
                    Some(RemoteInclude {
                        project: Some(project),
                        reference,
                        file: Some(file),
                    }),
                local: None,
                remote_url: None,
                basic: None,
                component: None,
            }) => {
                assert_eq!(want_project, project);
                assert_eq!(None, reference);
                assert_eq!(want_file, file);
            }
            _ => panic!("project file is invalid"),
        }
    }

    #[test]
    fn test_get_position_type_include_local() {
        let cnt = r#"
include:
  - project: myproject/name
    ref: 1.5.0
    file:
      - "/resources/ci-templates/mytemplate.yml"
  - local: ".my-local.yml"
  - remote: "https://myremote.com/template.yml"

job_one:
  image: alpine
  extends: .first
  stage: one
  variables:
    SEARCHED: no
    OTHER: yes
  needs:
    - job: job_one
"#;

        let treesitter = TreesitterImpl::new();
        let project_file = treesitter.get_position_type(
            cnt,
            Position {
                line: 6,
                character: 14,
            },
        );

        let want_path = "\".my-local.yml\"";
        match project_file {
            parser::PositionType::Include(IncludeInformation {
                remote: None,
                local: Some(Include { path }),
                remote_url: None,
                basic: None,
                component: None,
            }) => {
                assert_eq!(want_path, path);
            }
            _ => panic!("project file is invalid"),
        }
    }

    #[test]
    fn test_get_position_type_include_remote_url() {
        let cnt = r#"
    include:
      - project: myproject/name
        ref: 1.5.0
        file:
          - "/resources/ci-templates/mytemplate.yml"
      - local: ".my-local.yml"
      - remote: "https://myremote.com/template.yml"

    job_one:
      image: alpine
      extends: .first
      stage: one
      variables:
        SEARCHED: no
        OTHER: yes
      needs:
        - job: job_one
    "#;

        let treesitter = TreesitterImpl::new();
        let pos_type = treesitter.get_position_type(
            cnt,
            Position {
                line: 7,
                character: 20,
            },
        );

        let want_path = "\"https://myremote.com/template.yml\"";
        match pos_type {
            parser::PositionType::Include(IncludeInformation {
                remote: None,
                local: None,
                remote_url: Some(Include { path }),
                basic: None,
                component: None,
            }) => {
                assert_eq!(want_path, path);
            }
            _ => panic!("invalid type"),
        }
    }

    #[test]
    fn test_get_position_type_extend() {
        let cnt = r#"
include:
  - project: myproject/name
    ref: 1.5.0
    file:
      - "/resources/ci-templates/mytemplate.yml"
  - local: ".my-local.yml"
  - remote: "https://myremote.com/template.yml"

job_one:
  image: alpine
  extends: .first
  stage: one
  variables:
    SEARCHED: no
    OTHER: yes
  needs:
    - job: job_one
"#;

        let treesitter = TreesitterImpl::new();
        let pos_type = treesitter.get_position_type(
            cnt,
            Position {
                line: 11,
                character: 15,
            },
        );

        assert!(matches!(pos_type, parser::PositionType::Extend));
    }

    #[test]
    fn test_get_position_type_stage() {
        let cnt = r#"
include:
  - project: myproject/name
    ref: 1.5.0
    file:
      - "/resources/ci-templates/mytemplate.yml"
  - local: ".my-local.yml"
  - remote: "https://myremote.com/template.yml"

job_one:
  image: alpine
  extends: .first
  stage: one
  variables:
    SEARCHED: no
    OTHER: yes
  needs:
    - job: job_one
"#;

        let treesitter = TreesitterImpl::new();
        let pos_type = treesitter.get_position_type(
            cnt,
            Position {
                line: 12,
                character: 10,
            },
        );

        assert!(matches!(pos_type, parser::PositionType::Stage));
    }

    #[test]
    fn test_get_position_type_root_node() {
        let cnt = r#"
include:
  - project: myproject/name
    ref: 1.5.0
    file:
      - "/resources/ci-templates/mytemplate.yml"
  - local: ".my-local.yml"
  - remote: "https://myremote.com/template.yml"

job_one:
  image: alpine
  extends: .first
  stage: one
  variables:
    SEARCHED: no
    OTHER: yes
  needs:
    - job: job_one
"#;

        let treesitter = TreesitterImpl::new();
        let pos_type = treesitter.get_position_type(
            cnt,
            Position {
                line: 9,
                character: 4,
            },
        );

        assert!(matches!(pos_type, parser::PositionType::RootNode));
    }

    #[test]
    fn test_get_position_type_root_variable() {
        let cnt = r#"
    include:
      - project: myproject/name
        ref: 1.5.0
        file:
          - "/resources/ci-templates/mytemplate.yml"
      - local: ".my-local.yml"
      - remote: "https://myremote.com/template.yml"

    job_one:
      image: alpine
      extends: .first
      stage: one
      variables:
        SEARCHED: no
        OTHER: yes
      needs:
        - job: job_one
    "#;

        let treesitter = TreesitterImpl::new();
        let pos_type = treesitter.get_position_type(
            cnt,
            Position {
                line: 17,
                character: 17,
            },
        );

        let want_name = "job_one";
        match pos_type {
            parser::PositionType::Needs(NodeDefinition { name }) => {
                assert_eq!(want_name, name);
            }
            _ => panic!("invalid type"),
        }
    }

    #[test]
    fn test_get_position_type_rule_reference() {
        let cnt = r"
    .rules:job:
      rules:
        - test
    job_one:
      image: alpine
      extends: .first
      stage: one
      rules:
        - !reference ['.rules:job', rules]
      variables:
        SEARCHED: no
        OTHER: yes
      needs:
        - job: job_one
    ";

        let treesitter = TreesitterImpl::new();
        let pos_type = treesitter.get_position_type(
            cnt,
            Position {
                line: 9,
                character: 25,
            },
        );

        let want_node = ".rules:job";
        match pos_type {
            parser::PositionType::RuleReference(RuleReference { node }) => {
                assert_eq!(want_node, node);
            }
            _ => panic!("invalid type"),
        }
    }

    #[test]
    fn test_get_position_type_rule_reference_double_quote() {
        let cnt = r#"
    .rules:job:
      rules:
        - test
    job_one:
      image: alpine
      extends: .first
      stage: one
      rules:
        - !reference [".rules:job", rules]
      variables:
        SEARCHED: no
        OTHER: yes
      needs:
        - job: job_one
    "#;

        let treesitter = TreesitterImpl::new();
        let pos_type = treesitter.get_position_type(
            cnt,
            Position {
                line: 9,
                character: 25,
            },
        );

        let want_node = ".rules:job";
        match pos_type {
            parser::PositionType::RuleReference(RuleReference { node }) => {
                assert_eq!(want_node, node);
            }
            _ => panic!("invalid type"),
        }
    }

    #[test]
    fn test_get_all_multi_caches() {
        let cnt = r"
    job_one:
      image: alpine
      extends: .first
      stage: one
      cache:
        - key:
            files:
              - ./package.json
          paths:
            - ./node_modules
        - key:
            files:
              - ./package.json
          paths:
            - ./node_modules
      needs:
        - job: job_one
    ";

        let uri = "file://mocked";

        let treesitter = TreesitterImpl::new();
        let all_multi_caches = treesitter.get_all_multi_caches(uri, cnt);

        assert_eq!(1, all_multi_caches.len());
        assert_eq!(2, all_multi_caches[0].cache_items.len());
    }
}
