use std::collections::HashSet;
use lsp_types::Url;

use super::ParseResults;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkspaceId(pub String);

#[derive(Debug)]
pub enum IndexingState {
    New,
    InProgress,
    Completed,
    Failed(String),
}

#[derive(Debug)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub root_uri: Url,
    pub files_included: HashSet<Url>,
    pub parsed_data: Option<ParseResults>,
    pub indexing_state: IndexingState,
}

impl Workspace {
    pub fn new(root_uri: Url) -> Self {
        let id = WorkspaceId(root_uri.as_str().to_string());
        Workspace {
            id,
            root_uri,
            files_included: HashSet::new(),
            parsed_data: None,
            indexing_state: IndexingState::New,
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

        assert_eq!(workspace.id, WorkspaceId(root_uri.as_str().to_string()));
        assert_eq!(workspace.root_uri, root_uri);
        assert!(workspace.files_included.is_empty());
        assert!(workspace.parsed_data.is_none());
        match workspace.indexing_state {
            IndexingState::New => (),
            _ => panic!("Expected IndexingState::New"),
        }
    }
}
