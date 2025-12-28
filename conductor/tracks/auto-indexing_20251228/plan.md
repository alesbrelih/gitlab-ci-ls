# Implementation Plan - Track: auto-indexing

## Phase 1: Core Data Structures & Refactoring [checkpoint: e309860]
- [x] Task: Define `Workspace` and `WorkspaceId` structs e716d43
    - Create new module `src/gitlab_ci_ls_parser/workspace.rs`
    - Define `Workspace` struct with `root_uri`, `files_included`, `parsed_data`, etc.
    - Implement `Workspace::new(root_uri)`
- [x] Task: Refactor `LSPHandlers` to support Multi-Root ec5a04a
    - Modify `LSPHandlers` struct in `src/gitlab_ci_ls_parser/handlers.rs`
    - Replace single-state fields with `workspaces: Vec<Workspace>` (or `HashMap`)
    - **CRITICAL:** Implement a backward-compatible `get_active_workspace(uri)` helper to allow existing methods to function by returning the best-match workspace or a default one.
    - Update `new()` to initialize the empty collection.

## Phase 2: Intelligent Scanning & Fingerprinting
- [x] Task: Implement Content Fingerprinting 1d50af9
    - Create `src/gitlab_ci_ls_parser/fingerprint.rs`
    - Implement `is_gitlab_ci_file(content: &str) -> bool`
    - Add unit tests with sample Ansible, K8s, and GitLab CI files to verify accuracy.
- [x] Task: Implement Dependency Graph Builder e1bcfdd
    - Create `src/gitlab_ci_ls_parser/graph.rs`
    - Implement `build_dependency_graph(files: &[File]) -> (IncludeGraph, ReverseGraph)`
    - Implement `find_roots(graph, files) -> Vec<Uri>` using the fingerprint logic for orphans.

## Phase 3: Integration & Async Indexing
- [ ] Task: Implement `initialize_workspace`
    - In `LSPHandlers`, add `scan_workspace(root_dir)` method.
    - Use `glob` to find all YAML files.
    - Run the Dependency Graph Builder.
    - For each discovered Root, create a `Workspace` and trigger parsing.
- [ ] Task: Connect to LSP `initialize` event
    - Call `scan_workspace` during initialization.
    - Ensure it runs asynchronously (don't block init response).

## Phase 4: Context-Aware Resolution Features
- [ ] Task: Update `get_all_job_needs` / Completion
    - Update `handlers.rs` to use `find_workspaces_for(uri)` instead of assuming a single root.
    - Aggregate results if multiple workspaces are found.
- [ ] Task: Update "Find References"
    - Ensure it queries the reverse index of the correct workspace(s).

## Phase 5: Verification
- [ ] Task: Conductor - User Manual Verification 'Phase 4' (Protocol in workflow.md)
