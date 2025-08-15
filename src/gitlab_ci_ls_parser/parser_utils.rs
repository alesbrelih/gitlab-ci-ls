use std::env;

use super::GitlabElement;
use glob::glob;
use log::{error, info};
use temp_env::with_var;

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
            .rfind(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '[')
            .map_or(0, |index| index + 1);

        let end = line[char_index..]
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ']')
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
            .rfind(['$', '{'])
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

    pub fn find_path_at_cursor(line: &str, cursor_pos: usize) -> (String, String) {
        if cursor_pos >= line.len() {
            return (String::new(), String::new());
        }

        let mut current_component = String::new();
        let mut previous_components = String::new();

        let mut found_current = false;

        for i in (0..cursor_pos).rev() {
            let c = line.chars().nth(i).unwrap();
            let p = {
                if i == 0 {
                    'X'
                } else {
                    line.chars().nth(i - 1).unwrap()
                }
            };

            if p.is_whitespace() || c == '"' {
                break;
            }

            if c == '/' || c == '\\' {
                if found_current {
                    previous_components.insert(0, c); // insert at the beginning since we're moving backwards
                } else {
                    found_current = true;
                }
            } else if found_current {
                previous_components.insert(0, c); // insert at the beginning since we're moving backwards
            } else {
                current_component.insert(0, c); // insert at the beginning since we're moving backwards
            }
        }

        (current_component, previous_components)
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
            }
        }
        Err(anyhow::anyhow!("could not find component"))
    }

    pub fn extract_component_from_uri(
        uri: &str,
        git_remote_uris: &[String],
    ) -> anyhow::Result<ComponentInfo> {
        let mut component_parts = uri.split('/').collect::<Vec<&str>>();
        if component_parts.len() < 2 {
            return Err(anyhow::anyhow!(
                "invalid component uri structure; got: {uri}"
            ));
        }

        let mut host = component_parts.remove(0).to_string();
        // check if CI_SERVER_FQDN is being used. If so check if env variable is being set else set
        // to to git host
        if host.contains("$CI_SERVER_FQDN") {
            let ci_server_fqdn =
                env::var("CI_SERVER_FQDN").unwrap_or_else(|_| git_remote_uris[0].clone());

            host = host.replace("$CI_SERVER_FQDN", &ci_server_fqdn);
        }

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

    pub(crate) fn is_glob(uri: &str) -> bool {
        // TODO: kinda should work?
        uri.contains('*')
    }

    pub fn gitlab_style_glob(pattern: &str) -> Vec<std::path::PathBuf> {
        let mut results = Vec::new();
        let mut excludes = Vec::new();

        // Generate exclude list. Because when using **/* in gitlab
        // it means it should not match files in the current parent folder.. Only nested subfolders
        // So this way I can find files that I can exclude from actual glob pattern of **/* which
        // matches all
        if pattern.contains("**/*") {
            let new_pattern = pattern.replace("**/*", "*");

            let files = match glob(&new_pattern) {
                Ok(files) => files,
                Err(err) => {
                    error!("error matching files: {err}");
                    return vec![];
                }
            };

            excludes.extend(files.flatten());
        }

        // Replace gitlab custom pattern with valid one
        let mut pattern = pattern.to_string();
        if pattern.contains("**.") {
            pattern = pattern.replace("**", "**/*");
        }

        let files = match glob(&pattern) {
            Ok(files) => files,
            Err(err) => {
                error!("error matching files: {err}");
                return vec![];
            }
        };

        // Exclude current folder files if **/* was specified -> exlcudes were populated.
        results.extend(files.flatten().filter(|x| !excludes.contains(x)));

        results
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

        let got =
            match ParserUtils::extract_component_from_uri(component_uri, &["test".to_string()]) {
                Ok(c) => c,
                Err(err) => panic!("unable to extract; got: {err}"),
            };

        assert_eq!(got, want);
    }

    #[test]
    fn test_extract_component_from_uri_ci_server_fqdn_no_environment_variable() {
        let component_uri = "$CI_SERVER_FQDN/some-project/sub-project/component@1.0.0";
        let want = ComponentInfo {
            component: "component".to_string(),
            version: "1.0.0".to_string(),
            project: "some-project/sub-project".to_string(),
            host: "test-host-uri".to_string(),
        };

        let got = match ParserUtils::extract_component_from_uri(
            component_uri,
            &["test-host-uri".to_string()],
        ) {
            Ok(c) => c,
            Err(err) => panic!("unable to extract; got: {err}"),
        };

        assert_eq!(got, want);
    }

    #[test]
    fn test_extract_component_from_uri_ci_server_fqdn_with_environment_variable() {
        with_var("CI_SERVER_FQDN", Some("env-host-uri"), || {
            let component_uri = "$CI_SERVER_FQDN/some-project/sub-project/component@1.0.0";
            let want = ComponentInfo {
                component: "component".to_string(),
                version: "1.0.0".to_string(),
                project: "some-project/sub-project".to_string(),
                host: "fenv-host-uri".to_string(),
            };
            let got = match ParserUtils::extract_component_from_uri(
                component_uri,
                &["test-host-uri".to_string()],
            ) {
                Ok(c) => c,
                Err(err) => panic!("unable to extract; got: {err}"),
            };
            assert_eq!(got, want);
        });
    }

    #[test]
    fn test_find_path_at_cursor() {
        let line = "/test/please/here";
        let cursor = 14;
        let (path, parent) = ParserUtils::find_path_at_cursor(line, cursor);
        assert_eq!(path, "h");
        assert_eq!(parent, "/test/please");
    }

    #[test]
    fn test_find_path_at_cursor_quotes() {
        let line = r#""/test/please/here""#;
        let cursor = 15;
        let (path, parent) = ParserUtils::find_path_at_cursor(line, cursor);
        assert_eq!(path, "h");
        assert_eq!(parent, "/test/please");
    }
}
