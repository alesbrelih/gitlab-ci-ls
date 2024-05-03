use log::info;

use super::GitlabElement;

pub struct ParserUtils {}

#[derive(Debug, PartialEq, Clone)]
pub struct ComponentInfo {
    pub host: String,
    pub project: String,
    pub component: String,
    pub version: String,
}

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

    pub fn word_after_cursor(
        line: &str,
        char_index: usize,
        predicate: fn(c: char) -> bool,
    ) -> &str {
        if char_index >= line.len() {
            return "";
        }

        let start = char_index;

        let end = line[start..]
            .char_indices()
            .find(|&(_, c)| predicate(c))
            .map_or(line.len(), |(idx, _)| start + idx);

        &line[start..end]
    }

    pub fn remote_path_to_hash(uri: &str) -> String {
        crc64::crc64(0, uri.as_bytes()).to_string()
    }

    pub fn extract_variable(line: &str, char_index: usize) -> Option<&str> {
        if char_index >= line.len() {
            return None;
        }

        let start = line[..char_index]
            .rfind(|c: char| c == '$' || c == '{')
            .map_or(0, |index| index + 1);

        let end = line[char_index..]
            .find(|c: char| !c.is_alphabetic() && c != '_')
            .map_or(line.len(), |index| index + char_index);

        Some(&line[start..end])
    }

    pub fn get_component_dest_dir(cache_path: &str, component_info: &ComponentInfo) -> String {
        let components_path = format!("{cache_path}components/");
        format!(
            "{}{}/{}",
            components_path, component_info.project, component_info.version
        )
    }

    // Its used because component can be in four different places
    // TODO: test this
    pub fn get_component(repo_dest: &str, component_name: &str) -> anyhow::Result<GitlabElement> {
        let paths = vec![
            format!("{}/templates/{}.yml", repo_dest, component_name),
            format!("{}/templates/{}.yaml", repo_dest, component_name),
            format!("{}/templates/{}/template.yml", repo_dest, component_name),
            format!("{}/templates/{}/template.yaml", repo_dest, component_name),
        ];

        for p in paths {
            match std::fs::read_to_string(&p) {
                Ok(content) => {
                    return Ok(GitlabElement {
                        key: format!("file://{p}"),
                        content: Some(content),
                        uri: format!("file://{p}"),
                        range: super::Range {
                            start: super::LSPPosition {
                                line: 0,
                                character: 0,
                            },
                            end: super::LSPPosition {
                                line: 0,
                                character: 0,
                            },
                        },
                    });
                }
                Err(err) => {
                    info!("could not find component; path: {p}, got err: {err}");
                }
            };
        }
        Err(anyhow::anyhow!("could not find component"))
    }

    pub fn extract_component_from_uri(uri: &str) -> anyhow::Result<ComponentInfo> {
        let mut component_parts = uri.split('/').collect::<Vec<&str>>();
        if component_parts.len() < 2 {
            return Err(anyhow::anyhow!(
                "invalid component uri structure; got: {uri}"
            ));
        }

        let host = component_parts.remove(0);
        let Some(component) = component_parts.pop() else {
            return Err(anyhow::anyhow!(
                "could not get last element from component uri"
            ));
        };

        let component_identificator = component.split('@').collect::<Vec<&str>>();
        if component_identificator.len() != 2 {
            return Err(anyhow::anyhow!(
                "currently supported are only components with versions"
            ));
        }

        Ok(ComponentInfo {
            host: host.to_string(),
            component: component_identificator[0].to_string(),
            project: component_parts.join("/"),
            version: component_identificator[1].to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use core::panic;

    use super::*;

    #[test]
    fn test_extract_component_from_uri() {
        let component_uri = "gitlab.com/some-project/sub-project/component@1.0.0";
        let want = ComponentInfo {
            component: "component".to_string(),
            version: "1.0.0".to_string(),
            project: "some-project/sub-project".to_string(),
            host: "gitlab.com".to_string(),
        };

        let got = match ParserUtils::extract_component_from_uri(component_uri) {
            Ok(c) => c,
            Err(err) => panic!("unable to extract; got: {err}"),
        };

        assert_eq!(got, want);
    }
}
