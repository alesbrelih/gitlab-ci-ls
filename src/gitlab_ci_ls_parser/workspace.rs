use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use lsp_types::Url;

use super::{GitlabElement, GitlabFileElements, Component};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexingState {
    New,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug)]
pub struct Workspace {
    pub root_uri: Url,
    pub files_included: Mutex<HashSet<String>>,
    pub indexing_state: Mutex<IndexingState>,

    pub store: Mutex<HashMap<String, String>>,
    pub nodes: Mutex<HashMap<String, HashMap<String, GitlabElement>>>,
    pub nodes_ordered_list: Mutex<Vec<GitlabFileElements>>,
    pub stages: Mutex<HashMap<String, GitlabElement>>,
    pub stages_ordered_list: Mutex<Vec<String>>,
    pub variables: Mutex<HashMap<String, GitlabElement>>,
    pub components: Mutex<HashMap<String, Component>>,
}

impl Workspace {
    pub fn new(root_uri: Url) -> Self {
        Workspace {
            root_uri,
            files_included: Mutex::new(HashSet::new()),
            indexing_state: Mutex::new(IndexingState::New),
            store: Mutex::new(HashMap::new()),
            nodes: Mutex::new(HashMap::new()),
            nodes_ordered_list: Mutex::new(Vec::new()),
            stages: Mutex::new(HashMap::new()),
            stages_ordered_list: Mutex::new(Vec::new()),
            variables: Mutex::new(HashMap::new()),
            components: Mutex::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_new() {
        let root_uri = Url::parse("file:///root/.gitlab-ci.yml").unwrap();
        let workspace = Workspace::new(root_uri.clone());

        assert_eq!(workspace.root_uri, root_uri);
        assert!(workspace.files_included.lock().unwrap().is_empty());
        assert_eq!(*workspace.indexing_state.lock().unwrap(), IndexingState::New);
        assert!(workspace.store.lock().unwrap().is_empty());
           
        
        assert!(workspace.nodes.lock().unwrap().is_empty());
        assert!(workspace.nodes_ordered_list.lock().unwrap().is_empty());
        assert!(workspace.stages_ordered_list.lock().unwrap().is_empty());
        assert!(workspace.variables.lock().unwrap().is_empty());
        assert!(workspace.components.lock().unwrap().is_empty());
    }
}




