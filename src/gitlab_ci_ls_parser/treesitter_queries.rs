pub struct TreesitterQueries {}

impl TreesitterQueries {
    pub fn get_all_extends(extend_name: Option<&str>) -> String {
        let mut search = String::new();
        if extend_name.is_some() {
            let extend_name = extend_name.unwrap();
            search = format!(
                r#"(#any-of? @value "{extend_name}" "'{extend_name}'" "\"{extend_name}\"")"#,
            );
        }

        format!(
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
        )
    }

    pub fn get_root_node(node_key: &str) -> String {
        format!(
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
        )
    }

    pub fn get_root_node_key(node_key: &str) -> String {
        format!(
            r#"
        (
            stream(
                document(
                    block_node(
                        block_mapping(
                            block_mapping_pair
                                key: (flow_node(plain_scalar(string_scalar)@key))
                        )
                    )
                )
            )
            (#eq? @key "{node_key}")
        )
        "#
        )
    }

    pub fn get_all_root_nodes() -> String {
        r"
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
        "
        .to_string()
    }

    pub fn get_root_variables() -> String {
        r#"
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
        "#
        .to_string()
    }

    pub fn get_stage_definitions() -> String {
        r#"
        (
            block_mapping_pair
            key: (flow_node(plain_scalar(string_scalar) @key))
            value: (block_node(block_sequence(block_sequence_item(flow_node(plain_scalar(string_scalar) @value)))))

            (#eq? @key "stages")
        )
        "#.to_string()
    }

    pub fn get_all_stages(stage: Option<&str>) -> String {
        let mut search = String::new();
        if stage.is_some() {
            search = format!("(#eq? @value \"{}\")", stage.unwrap());
        }

        format!(
            r#"
        (
            document (
                block_node (
                    block_mapping (
                        block_mapping_pair
                            value: (
                                block_node (
                                    block_mapping (
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

                                    )
                                )
                            )
                    )
                )
            )
            (#eq? @key "stage")
            {search}
        )
        "#
        )
    }

    pub fn get_all_rule_references(rule: Option<&str>) -> String {
        let mut search = String::new();
        if rule.is_some() {
            let rule = rule.unwrap();
            search =
                format!(r#"(#any-of? @rule_reference_value "{rule}" "'{rule}'" "\"{rule}\"")"#,);
        }

        format!(
            r#"
        (
            block_mapping_pair
            key: (flow_node) @rule_reference_key
            value: (
                block_node(
                    block_sequence(
                        block_sequence_item(
                            flow_node
                            (
                              (tag)@rule_reference_tag
                              (
                                flow_sequence(
                                    (flow_node[(single_quote_scalar)(double_quote_scalar)])@rule_reference_value
                                    (_)?
                                )
                               )
                            )
                        )
                    )
                )
            )
            (#eq? @rule_reference_key "rules")
            (#eq? @rule_reference_tag "!reference")
            {search}
        )
        "#
        )
    }

    #[allow(clippy::too_many_lines)]
    pub fn get_position_type() -> String {
        let search_extends = r#"
            (
                block_mapping_pair
                key: (flow_node) @keyextends
                value: [
                    (flow_node(plain_scalar(string_scalar))) @extends
                    (block_node(block_sequence(block_sequence_item) @extends))
                ]
                (#eq? @keyextends "extends")
            )
        "#;

        let search_stages = r#"
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
                            plain_scalar(string_scalar) @keystage
                        )
                    )
                    value: (block_node(block_sequence(block_sequence_item)@stage ))
                (#eq? @keystage "stages")
            )
        "#;

        let search_variables = r#"
            (
                block_mapping_pair
                key: (
                    flow_node(
                        plain_scalar(string_scalar)  @keyvariable
                    )
                )
                value:
                (
                    block_node(
                        block_mapping(block_mapping_pair
                            value:
                                [
                                    (flow_node(flow_sequence(flow_node) ))
                                    (flow_node)
                                ] @variable
                        )
                    )
                )
                (#eq? @keyvariable "image")
            )
            (
                block_mapping_pair
                key: (
                    flow_node(
                        plain_scalar(string_scalar)  @keyvariable
                    )
                )
                value:
                (
                    block_node(
                        block_mapping(block_mapping_pair
                            value:
                                [
                                    (flow_node(flow_sequence(flow_node) ))
                                    (flow_node)
                                ] @variable
                        )
                    )
                )
                (#eq? @keyvariable "variables")
            )
            (
                block_mapping_pair
                key: (
                    flow_node(
                        plain_scalar(string_scalar)  @keyvariable
                    )
                )
                value:
                (
                    block_node(
                        block_sequence(block_sequence_item) @variable
                    )
                )
                (#eq? @keyvariable "before_script")
            )
            (
                block_mapping_pair
                key: (
                    flow_node(
                        plain_scalar(string_scalar)  @keyvariable
                    )
                )
                value:
                (
                    block_node(
                        block_sequence(block_sequence_item) @variable
                    )
                )
                (#eq? @keyvariable "script")
            )
            (
                block_mapping_pair
                key: (
                    flow_node(
                        plain_scalar(string_scalar)  @keyvariable
                    )
                )
                value:
                (
                    block_node(
                        block_sequence(block_sequence_item) @variable
                    )
                )
                (#eq? @keyvariable "after_script")
            )
            (
                block_mapping_pair
                key: (
                    flow_node(
                        plain_scalar(string_scalar)  @keyvariable
                    )
                )
                value:
                (
                    block_node(
                        block_sequence(
                            block_sequence_item(
                                block_node(
                                    block_mapping(
                                        block_mapping_pair
                                            key: (flow_node(plain_scalar))
                                            value: (flow_node)@variable
                                    )
                                )
                            )
                        )
                    )
                )
                (#eq? @keyvariable "rules")
            )
            (
                block_mapping_pair
                key: (
                    flow_node(
                        plain_scalar(string_scalar)  @keyvariable
                    )
                )
                value:
                (
                    block_node(
                        block_mapping(block_mapping_pair
                            value:
                            (block_node(block_sequence)@variable)
                        )
                    )
                )
                (#eq? @keyvariable "parallel")
            )
        "#;

        let search_root_node = r"
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
        ";

        let search_local_include = r#"
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
        "#;

        let search_basic_include = r#"
            (
                stream(
                    document(
                        block_node(
                            block_mapping(
                                block_mapping_pair
                                    key: (flow_node(plain_scalar(string_scalar)@basic_include_key))
                                    value: (
                                        block_node(
                                            block_sequence(
                                                block_sequence_item(
                                                flow_node
                                                ) @basic_include_value
                                            )
                                        )
                                    )
                                )
                            )
                        )
                    )
                (#eq? @local_include_key "include")
            )
        "#;

        let search_project_includes = r#"
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
                                                                value: [(block_node(block_sequence(block_sequence_item(flow_node)@file_value)))(flow_node)@file_value]
                                                            )
                                                        )
                                                    ]
                                                )
                                            )@remote_include_item
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

        // component_input_error is when user start typing new key and its an error because it
        // needs to be a map
        let search_component_include = r#"
        (
            block_sequence_item(
                block_node(
                    block_mapping(
                        (block_mapping_pair
                            key: (flow_node(plain_scalar(string_scalar)@component_include_key))
                            value: (flow_node) @component_uri
                        )
                        (block_mapping_pair
                            key: (flow_node(plain_scalar(string_scalar)@component_inputs_key))
                            value: [(block_node(block_mapping
                            [
                                (
                                  block_mapping_pair
                                    key: (flow_node(plain_scalar(string_scalar)@component_input))
                                    value:
                                    [
                                        (flow_node)@component_input_value_plain
                                        (block_node)@component_input_value_block
                                    ]?
                                )*
                                (ERROR(flow_node(plain_scalar(string_scalar)@component_input_error)))*
                            ]
                            ))(flow_node)@component_input]
                        )
                    )
                ) @full_component
            )
            (#eq? @component_include_key "component")
            (#eq? @component_inputs_key "inputs")
        )
        "#;

        let search_job_needs = r#"
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
        "#;

        let search_remote_urls = r#"
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

        // (_)? means optional any node
        let search_rule_references = r#"
        (
            block_mapping_pair
            key: (flow_node) @rule_reference_key
            value: (
                block_node(
                    block_sequence(
                        block_sequence_item(
                            flow_node
                            (
                              (tag)@rule_reference_tag
                              (
                                flow_sequence(
                                    (flow_node[(single_quote_scalar)(double_quote_scalar)])@rule_reference_value
                                    (_)?
                                )
                               )
                            )
                        )
                    )
                )
            )
            (#eq? @rule_reference_key "rules")
            (#eq? @rule_reference_tag "!reference")
        )
        "#;

        format!(
            r#"
            {search_rule_references}
            {search_extends}
            {search_stages}
            {search_variables}
            {search_root_node}
            {search_local_include}
            {search_project_includes}
            {search_component_include}
            {search_job_needs}
            {search_remote_urls}
            {search_basic_include}
        "#
        )
    }

    pub fn get_all_job_needs(needs_name: Option<&str>) -> String {
        let mut search = String::new();
        if needs_name.is_some() {
            let needs_name = needs_name.unwrap();
            search = format!(
                r#"(#any-of? @needs_job_value "{needs_name}" "'{needs_name}'" "\"{needs_name}\"")"#,
            );
        }

        format!(
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
        )
    }

    pub fn get_root_node_at_position() -> String {
        r"
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
        "
        .to_string()
    }

    pub fn get_job_variable_definition(job_name: &str, variable_name: &str) -> String {
        format!(
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
        )
    }

    pub fn get_component_spec_inputs() -> String {
        r#"
           (
               stream(
                   document(
                       block_node(
                           block_mapping(
                               block_mapping_pair
                               key: (flow_node(plain_scalar(string_scalar)@key))
                           )
                       )
                   ) @spec
               )
               (#eq? @key "spec")
           )
           "#
        .to_string()
    }

    pub fn get_all_components() -> String {
        r#"
        (
            block_sequence_item(
                block_node(
                    block_mapping(
                        (block_mapping_pair
                            key: (flow_node(plain_scalar(string_scalar)@component_include_key))
                            value: (flow_node) @component_uri
                        )
                        (block_mapping_pair
                            key: (flow_node(plain_scalar(string_scalar)@component_inputs_key))
                            value: (block_node(block_mapping(
                              block_mapping_pair
                                key: (flow_node(plain_scalar(string_scalar)@component_input))
                                value:
                                [
                                    (flow_node)@component_input_value_plain
                                    (block_node)@component_input_value_block
                                ]?
                            )*))
                        )
                    )
                ) @full_component
            )
            (#eq? @component_include_key "component")
            (#eq? @component_inputs_key "inputs")
        )
        "#
        .to_string()
    }

    pub fn get_all_caches() -> String {
        r#"
        (
            block_mapping (
            block_mapping_pair
                key: (flow_node (plain_scalar(string_scalar)@cache_key))
                value: (block_node(block_sequence(block_sequence_item)+@cache_item))

            (#eq? @cache_key "cache")
            ) @cache_node
        )
        "#
        .to_string()
    }
}
