use std::collections::{HashMap, HashSet};
use lsp_types::Url;
use super::IncludeItem;
use super::fingerprint::is_gitlab_ci_file;

#[derive(Debug, Default)]
pub struct ProjectGraph {
    pub includes: HashMap<String, HashSet<String>>,
    pub included_by: HashMap<String, HashSet<String>>,
}

impl ProjectGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_inclusion(&mut self, parent: &str, child: &str) {
        self.includes
            .entry(parent.to_string())
            .or_default()
            .insert(child.to_string());
        self.included_by
            .entry(child.to_string())
            .or_default()
            .insert(parent.to_string());
    }
}

pub fn build_graph(files: &HashMap<String, String>) -> ProjectGraph {
    let mut graph = ProjectGraph::new();

    for (uri_str, content) in files {
        let uri = match Url::parse(uri_str) {
            Ok(u) => u,
            Err(_) => continue,
        };

        let yaml: serde_yaml::Value = match serde_yaml::from_str(content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(include_val) = yaml.get("include") {
            // GitLab allows 'include' to be a single string, a single object, or an array of both.
            let items: Vec<serde_yaml::Value> = if let Some(seq) = include_val.as_sequence() {
                seq.clone()
            } else {
                vec![include_val.clone()]
            };

            for item_val in items {
                let item: Result<IncludeItem, _> = serde_yaml::from_value(item_val);
                if let Ok(item) = item {
                    match item {
                        IncludeItem::Local(l) => {
                            if let Ok(child_uri) = uri.join(&l.local) {
                                graph.add_inclusion(uri_str, child_uri.as_str());
                            }
                        }
                        IncludeItem::Basic(b) => {
                            if Url::parse(&b).is_err() {
                                if let Ok(child_uri) = uri.join(&b) {
                                    graph.add_inclusion(uri_str, child_uri.as_str());
                                }
                            }
                        }
                        _ => {} // Ignore other include types for now
                    }
                }
            }
        }
    }

    graph
}

pub fn find_roots(graph: &ProjectGraph, files: &HashMap<String, String>) -> Vec<String> {
    let mut roots = Vec::new();

    for (uri_str, content) in files {
        // Rule 1: Strict Name
        if uri_str.ends_with(".gitlab-ci.yml") || uri_str.ends_with(".gitlab-ci.yaml") {
            roots.push(uri_str.clone());
            continue;
        }

        // Rule 2: Fingerprint + Orphan
        if is_gitlab_ci_file(content) {
            let is_included = graph
                .included_by
                .get(uri_str)
                .map_or(false, |parents| !parents.is_empty());

            if !is_included {
                roots.push(uri_str.clone());
            }
        }
    }

    roots.dedup();
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_graph_basic() {
        let mut files = HashMap::new();
        files.insert(
            "file:///root/.gitlab-ci.yml".to_string(),
            "include:\n  - local: 'common.yml'".to_string(),
        );
        files.insert("file:///root/common.yml".to_string(), "job: { script: 'ls' }".to_string());

        let graph = build_graph(&files);
        assert!(graph.includes["file:///root/.gitlab-ci.yml"].contains("file:///root/common.yml"));
        assert!(graph.included_by["file:///root/common.yml"].contains("file:///root/.gitlab-ci.yml"));
    }

    #[test]
    fn test_find_roots() {
        let mut files = HashMap::new();
        files.insert(
            "file:///root/.gitlab-ci.yml".to_string(),
            "include: 'common.yml'".to_string(),
        );
        files.insert("file:///root/common.yml".to_string(), "job: { script: 'ls' }".to_string());
        files.insert(
            "file:///root/template.yml".to_string(),
            "job_template: { script: 'echo' }".to_string(),
        );
        files.insert("file:///root/random.txt".to_string(), "hello world".to_string());

        let graph = build_graph(&files);
        let roots = find_roots(&graph, &files);

        assert!(roots.contains(&"file:///root/.gitlab-ci.yml".to_string()));
        assert!(roots.contains(&"file:///root/template.yml".to_string()));
        assert!(!roots.contains(&"file:///root/common.yml".to_string()));
        assert!(!roots.contains(&"file:///root/random.txt".to_string()));
    }
}
