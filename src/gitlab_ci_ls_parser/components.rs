//! Logic around GitLab CI/CD components that doesn't depend on LSP state.
//!
//! Covers both sides of a component:
//! - the template (`spec:` header + `$[[ inputs.x ]]` interpolation)
//! - the caller (`include: - component: ...` with `inputs:`)

use log::warn;

use super::{
    yaml_value_to_inline_string, ComponentInput, ComponentSpec, ComponentSpecInput,
    ParsedComponentSpec,
};

pub const INPUTS_CONTEXT: &str = "inputs";
pub const COMPONENT_CONTEXT: &str = "component";
pub const LATEST_VERSION: &str = "~latest";

/// Parses the `spec:` header document of a component template.
pub fn parse_component_spec(spec_yaml: &str) -> anyhow::Result<ParsedComponentSpec> {
    let spec: ComponentSpec = serde_yaml::from_str(spec_yaml)?;
    let Some(header) = spec.spec else {
        return Ok(ParsedComponentSpec::default());
    };

    let inputs = header
        .inputs
        .unwrap_or_default()
        .into_iter()
        .map(|(key, value)| {
            let key = match key {
                serde_yaml::Value::String(s) => s,
                other => yaml_value_to_inline_string(&other),
            };

            // `my_input:` without any attributes is a required string input
            let definition: ComponentSpecInput = if value.is_null() {
                ComponentSpecInput::default()
            } else {
                serde_yaml::from_value(value).unwrap_or_else(|err| {
                    warn!("could not parse definition of component input {key}; got err: {err}");
                    ComponentSpecInput::default()
                })
            };

            ComponentInput {
                key,
                default: definition.default,
                description: definition.description,
                options: definition
                    .options
                    .map(|options| options.iter().map(yaml_value_to_inline_string).collect()),
                regex: definition.regex,
                prop_type: definition.type_,
                has_rules: definition.rules.is_some(),
                ..Default::default()
            }
        })
        .collect();

    Ok(ParsedComponentSpec {
        inputs,
        component_context: header.component.unwrap_or_default(),
    })
}

/// `$[[ ... ]]` block found in a single line. Offsets are byte offsets inside the line.
#[derive(Debug, Clone, PartialEq)]
pub struct Interpolation {
    pub start: usize,
    pub content_start: usize,
    // exclusive; points at `]]` or the end of line when the block isn't closed yet
    pub content_end: usize,
    pub closed: bool,
}

impl Interpolation {
    pub fn content<'a>(&self, line: &'a str) -> &'a str {
        &line[self.content_start..self.content_end]
    }
}

/// Reference like `inputs.stage` inside an interpolation block.
#[derive(Debug, Clone, PartialEq)]
pub struct InterpolationReference {
    pub context: String,
    pub name: String,
    // byte offset of the context (`inputs`)
    pub start: usize,
    // byte offset of the name (`stage`)
    pub name_start: usize,
    // exclusive byte offset of the end of the name
    pub end: usize,
}

pub fn find_interpolations(line: &str) -> Vec<Interpolation> {
    let mut interpolations = vec![];
    let mut offset = 0;

    while let Some(idx) = line[offset..].find("$[[") {
        let start = offset + idx;
        let content_start = start + 3;
        let closing = line[content_start..].find("]]");
        let content_end = closing.map_or(line.len(), |end| content_start + end);

        interpolations.push(Interpolation {
            start,
            content_start,
            content_end,
            closed: closing.is_some(),
        });

        offset = (content_end + 2).min(line.len());
        if offset >= line.len() {
            break;
        }
    }

    interpolations
}

fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

/// Returns all `<context>.<name>` references used in interpolation blocks on the line.
/// Only the leading expression is a reference; everything after `|` are functions.
pub fn find_interpolation_references(line: &str) -> Vec<InterpolationReference> {
    find_interpolations(line)
        .into_iter()
        .filter_map(|interpolation| {
            let expression = interpolation
                .content(line)
                .split('|')
                .next()
                .unwrap_or_default();
            let leading_ws = expression.len() - expression.trim_start().len();
            let expression = expression.trim();

            let (context, name) = expression.split_once('.')?;
            if context.is_empty()
                || name.is_empty()
                || !context.chars().all(is_identifier_char)
                || !name.chars().all(is_identifier_char)
            {
                return None;
            }

            let start = interpolation.content_start + leading_ws;
            let name_start = start + context.len() + 1;

            Some(InterpolationReference {
                context: context.to_string(),
                name: name.to_string(),
                start,
                name_start,
                end: name_start + name.len(),
            })
        })
        .collect()
}

pub fn interpolation_reference_at(line: &str, cursor: usize) -> Option<InterpolationReference> {
    find_interpolation_references(line)
        .into_iter()
        .find(|r| r.start <= cursor && cursor <= r.end)
}

/// Function applied in an interpolation block, e.g. `truncate` in
/// `$[[ inputs.x | truncate(0, 8) ]]`. Offsets are byte offsets of the function name.
#[derive(Debug, Clone, PartialEq)]
pub struct InterpolationFunction {
    pub name: String,
    pub start: usize,
    pub end: usize,
}

/// Returns functions used in closed interpolation blocks on the line.
pub fn find_interpolation_functions(line: &str) -> Vec<InterpolationFunction> {
    let mut functions = vec![];

    for interpolation in find_interpolations(line).into_iter().filter(|i| i.closed) {
        let mut offset = interpolation.content_start;

        for (idx, segment) in interpolation.content(line).split('|').enumerate() {
            if idx > 0 {
                let trimmed = segment.trim_start();
                let name_len = trimmed
                    .find(|c| !is_identifier_char(c))
                    .unwrap_or(trimmed.len());

                if name_len > 0 {
                    let start = offset + (segment.len() - trimmed.len());
                    functions.push(InterpolationFunction {
                        name: trimmed[..name_len].to_string(),
                        start,
                        end: start + name_len,
                    });
                }
            }

            offset += segment.len() + 1;
        }
    }

    functions
}

#[derive(Debug, Clone, PartialEq)]
pub enum InterpolationCompletion {
    // `$[[ inp|`
    Context { typed: String },
    // `$[[ inputs.st|`
    Name { context: String, typed: String },
    // `$[[ inputs.stage | exp|`
    Function { typed: String },
}

impl InterpolationCompletion {
    pub fn typed(&self) -> &str {
        match self {
            InterpolationCompletion::Context { typed }
            | InterpolationCompletion::Name { typed, .. }
            | InterpolationCompletion::Function { typed } => typed,
        }
    }
}

/// Figures out what should be completed when cursor is inside of an interpolation block.
pub fn interpolation_completion_at(line: &str, cursor: usize) -> Option<InterpolationCompletion> {
    let interpolation = find_interpolations(line)
        .into_iter()
        .find(|i| i.content_start <= cursor && cursor <= i.content_end)?;

    let before = line.get(interpolation.content_start..cursor)?;

    if let Some((_, function)) = before.rsplit_once('|') {
        let typed = function.trim_start();
        if !typed.chars().all(is_identifier_char) {
            return None;
        }

        return Some(InterpolationCompletion::Function {
            typed: typed.to_string(),
        });
    }

    let expression = before.trim_start();
    if expression.contains(char::is_whitespace) {
        return None;
    }

    match expression.split_once('.') {
        Some((context, typed)) => {
            if !typed.chars().all(is_identifier_char) {
                return None;
            }

            Some(InterpolationCompletion::Name {
                context: context.to_string(),
                typed: typed.to_string(),
            })
        }
        None => Some(InterpolationCompletion::Context {
            typed: expression.to_string(),
        }),
    }
}

/// Length of the identifier that continues after the cursor, used to replace the whole word.
pub fn identifier_len_after(line: &str, cursor: usize) -> usize {
    line.get(cursor..).map_or(0, |rest| {
        rest.find(|c| !is_identifier_char(c)).unwrap_or(rest.len())
    })
}

/// Validates caller provided input value against the input `type`.
/// `raw_value` is the value as written in the source (including quotes).
/// Returns an error message when the value can't be valid.
pub fn validate_input_type(input_type: &str, raw_value: &str, is_block: bool) -> Option<String> {
    let value = raw_value.trim();
    let quoted = value.starts_with('"') || value.starts_with('\'');
    // values using interpolation/variables are resolved at runtime
    if value.contains("$[[") {
        return None;
    }

    let is_array = value.starts_with('[') || (is_block && value.starts_with('-'));

    let valid = match input_type {
        "number" => !is_block && !quoted && value.parse::<f64>().is_ok(),
        "boolean" => !is_block && !quoted && (value == "true" || value == "false"),
        "array" => is_array,
        "string" => !is_array,
        _ => true,
    };

    if valid {
        None
    } else {
        Some(format!(
            "Invalid input value. Expected a value of type `{input_type}`."
        ))
    }
}

/// Strict `MAJOR.MINOR.PATCH` (CI/CD catalog only publishes such versions).
pub fn parse_semver(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;

    if parts.next().is_some() {
        return None;
    }

    Some((major, minor, patch))
}

/// Version references that point to a moving target and need to be resolved against
/// existing tags: `~latest`, `1` (latest 1.x.x) and `1.2` (latest 1.2.x).
pub fn is_partial_version(version: &str) -> bool {
    if version == LATEST_VERSION {
        return true;
    }

    let parts: Vec<&str> = version.split('.').collect();
    (1..=2).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Resolves `~latest`, `MAJOR` or `MAJOR.MINOR` to the highest matching semver tag.
pub fn resolve_version(requested: &str, tags: &[String]) -> Option<String> {
    let prefix: Vec<u64> = if requested == LATEST_VERSION {
        vec![]
    } else {
        requested
            .split('.')
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?
    };

    tags.iter()
        .filter_map(|tag| parse_semver(tag).map(|v| (v, tag)))
        .filter(|((major, minor, _), _)| match prefix.as_slice() {
            [] => true,
            [m] => major == m,
            [m, n] => major == m && minor == n,
            _ => false,
        })
        .max_by_key(|(v, _)| *v)
        .map(|(_, tag)| tag.clone())
}

/// Parses output of `git ls-remote --tags --refs` into tag names.
pub fn parse_ls_remote_tags(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter_map(|reference| reference.strip_prefix("refs/tags/"))
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_component_spec() {
        let spec = r"spec:
  component: [name, version]
  inputs:
    website:
    stage:
      default: test
      description: Stage of the job
    replicas:
      type: number
      default: 1
    level:
      options: [1, 2, 3]
    conditional:
      rules:
        - if: $[[ inputs.stage ]] == 'test'
          default: x
";

        let parsed = parse_component_spec(spec).unwrap();
        let keys: Vec<&str> = parsed.inputs.iter().map(|i| i.key.as_str()).collect();
        // declaration order is kept
        assert_eq!(
            keys,
            vec!["website", "stage", "replicas", "level", "conditional"]
        );
        assert_eq!(parsed.component_context, vec!["name", "version"]);

        let website = &parsed.inputs[0];
        assert!(website.is_required());
        assert_eq!(website.input_type(), "string");

        let stage = &parsed.inputs[1];
        assert!(!stage.is_required());
        assert_eq!(stage.description.as_deref(), Some("Stage of the job"));

        let replicas = &parsed.inputs[2];
        assert_eq!(replicas.input_type(), "number");
        assert_eq!(replicas.default_as_string().as_deref(), Some("1"));

        let level = &parsed.inputs[3];
        assert_eq!(
            level.options,
            Some(vec!["1".to_string(), "2".to_string(), "3".to_string()])
        );

        assert!(!parsed.inputs[4].is_required());
    }

    #[test]
    fn test_parse_component_spec_without_inputs() {
        let parsed = parse_component_spec("spec:\n  component: [name]\n").unwrap();
        assert!(parsed.inputs.is_empty());

        let parsed = parse_component_spec("spec:\n  inputs:\n").unwrap();
        assert!(parsed.inputs.is_empty());

        let parsed = parse_component_spec("spec:\n").unwrap();
        assert!(parsed.inputs.is_empty());
    }

    #[test]
    fn test_find_interpolation_references() {
        let line = r#"  script: echo "$[[ inputs.stage ]] $[[component.version]] $[[ inputs.name | expand_vars ]]""#;

        let refs = find_interpolation_references(line);
        assert_eq!(refs.len(), 3);

        assert_eq!(refs[0].context, "inputs");
        assert_eq!(refs[0].name, "stage");
        assert_eq!(&line[refs[0].start..refs[0].end], "inputs.stage");
        assert_eq!(&line[refs[0].name_start..refs[0].end], "stage");

        assert_eq!(refs[1].context, "component");
        assert_eq!(&line[refs[1].start..refs[1].end], "component.version");

        assert_eq!(refs[2].name, "name");
    }

    #[test]
    fn test_find_interpolation_functions() {
        let line =
            "  script: $[[ inputs.a | expand_vars | truncate(0, 8) ]] $[[ inputs.b | unclosed";

        let functions = find_interpolation_functions(line);
        let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(vec!["expand_vars", "truncate"], names);
        assert_eq!(&line[functions[1].start..functions[1].end], "truncate");
    }

    #[test]
    fn test_interpolation_reference_at() {
        let line = "  stage: $[[ inputs.stage ]]";
        let cursor = line.find("stage ]]").unwrap() + 2;

        let reference = interpolation_reference_at(line, cursor).unwrap();
        assert_eq!(reference.name, "stage");

        assert!(interpolation_reference_at(line, 3).is_none());
    }

    #[test]
    fn test_interpolation_completion_at() {
        let line = "  stage: $[[ inputs.st";
        assert_eq!(
            interpolation_completion_at(line, line.len()),
            Some(InterpolationCompletion::Name {
                context: "inputs".to_string(),
                typed: "st".to_string()
            })
        );

        let line = "  stage: $[[ inp ]]";
        let cursor = line.find(" ]]").unwrap();
        assert_eq!(
            interpolation_completion_at(line, cursor),
            Some(InterpolationCompletion::Context {
                typed: "inp".to_string()
            })
        );

        let line = "  stage: $[[ inputs.stage | exp ]]";
        let cursor = line.find(" ]]").unwrap();
        assert_eq!(
            interpolation_completion_at(line, cursor),
            Some(InterpolationCompletion::Function {
                typed: "exp".to_string()
            })
        );

        let line = "  stage: test";
        assert_eq!(interpolation_completion_at(line, line.len()), None);
    }

    #[test]
    fn test_validate_input_type() {
        assert!(validate_input_type("number", "5", false).is_none());
        assert!(validate_input_type("number", "1.5", false).is_none());
        assert!(validate_input_type("number", "\"5\"", false).is_some());
        assert!(validate_input_type("number", "five", false).is_some());

        assert!(validate_input_type("boolean", "true", false).is_none());
        assert!(validate_input_type("boolean", "yes", false).is_some());
        assert!(validate_input_type("boolean", "'true'", false).is_some());

        assert!(validate_input_type("array", "[a, b]", false).is_none());
        assert!(validate_input_type("array", "- a\n- b", true).is_none());
        assert!(validate_input_type("array", "a", false).is_some());

        assert!(validate_input_type("string", "hello", false).is_none());
        assert!(validate_input_type("string", "|\n  multi\n  line", true).is_none());
        assert!(validate_input_type("string", "[a]", false).is_some());

        assert!(validate_input_type("number", "$[[ inputs.x ]]", false).is_none());
    }

    #[test]
    fn test_resolve_version() {
        let tags = vec![
            "1.0.0".to_string(),
            "1.2.0".to_string(),
            "1.10.1".to_string(),
            "2.0.0".to_string(),
            "2.1.0-rc1".to_string(),
            "main-snapshot".to_string(),
        ];

        assert_eq!(resolve_version("~latest", &tags).as_deref(), Some("2.0.0"));
        assert_eq!(resolve_version("1", &tags).as_deref(), Some("1.10.1"));
        assert_eq!(resolve_version("1.2", &tags).as_deref(), Some("1.2.0"));
        assert_eq!(resolve_version("3", &tags), None);
    }

    #[test]
    fn test_is_partial_version() {
        assert!(is_partial_version("~latest"));
        assert!(is_partial_version("1"));
        assert!(is_partial_version("1.2"));
        assert!(!is_partial_version("1.2.3"));
        assert!(!is_partial_version("main"));
        assert!(!is_partial_version("1."));
    }

    #[test]
    fn test_parse_ls_remote_tags() {
        let output = "abc123\trefs/tags/1.0.0\ndef456\trefs/tags/2.0.0\n";
        assert_eq!(parse_ls_remote_tags(output), vec!["1.0.0", "2.0.0"]);
    }
}
