use std::{collections::HashMap, env, fs, path::Path};

use git2::{Cred, RemoteCallbacks};
use log::{debug, error, info};
use lsp_types::Url;
use tree_sitter::{Query, QueryCursor};
use tree_sitter_yaml::language;
use yaml_rust::{yaml::Hash, Yaml, YamlEmitter, YamlLoader};

use crate::{GitlabExtend, GitlabFile, GitlabRootNode, LSPPosition, Range};

pub struct Parser {
    cache_path: String,
    package_map: HashMap<String, String>,
    remote_urls: Vec<String>,
}

pub struct ParserUtils {}

impl ParserUtils {
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

    pub fn word_before_cursor(line: &str, char_index: usize) -> &str {
        if char_index == 0 || char_index > line.len() {
            return "";
        }

        let start = line[..char_index]
            .rfind(|c: char| c.is_whitespace())
            .map_or(0, |index| index + 1);

        if start == char_index {
            return "";
        }

        &line[start..char_index]
    }

    pub fn find_position(content: &str, word: &str) -> Option<Range> {
        for (line_num, line) in content.lines().enumerate() {
            if line.starts_with(word) {
                return Some(Range {
                    start: LSPPosition {
                        line: line_num as u32,
                        character: 0,
                    },
                    end: LSPPosition {
                        line: line_num as u32,
                        character: word.len() as u32,
                    },
                });
            }
        }

        None
    }

    pub fn get_root_node(content: &str, node_key: &str) -> Option<(Yaml, Yaml)> {
        let documents = match YamlLoader::load_from_str(content) {
            Ok(_documents) => _documents,
            Err(err) => {
                error!("parsing yaml from: {:?} got: {:?}", content, err);
                return None;
            }
        };

        let content = &documents[0];

        if let Yaml::Hash(root) = content {
            for (key, value) in root {
                if let Yaml::String(key_str) = key {
                    if key_str.as_str().eq(node_key) {
                        return Some((key.clone(), value.clone()));
                    }
                }
            }
        }

        None
    }

    fn get_all_root_nodes(content: &str) -> Vec<GitlabRootNode> {
        let mut nodes: Vec<GitlabRootNode> = vec![];

        let documents = match YamlLoader::load_from_str(content) {
            Ok(_documents) => _documents,
            Err(err) => {
                error!("parsing yaml from: {:?} got: {:?}", content, err);

                return nodes;
            }
        };

        let content = &documents[0];

        if let Yaml::Hash(root) = content {
            for (key, value) in root {
                let mut description = String::new();
                let mut hash = Hash::new();

                hash.insert(key.clone(), value.clone());

                let mut emitter = YamlEmitter::new(&mut description);
                emitter.dump(&Yaml::Hash(hash)).unwrap();

                nodes.push(GitlabRootNode {
                    key: key.as_str().unwrap().into(),
                    description,
                })
            }
        }

        nodes
    }

    pub fn get_all_extends(uri: String, content: &str) -> Vec<GitlabExtend> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(tree_sitter_yaml::language())
            .expect("Error loading YAML grammar");

        let query_source = r#"
        (
            block_mapping_pair
            key: (flow_node) @key2
            value: [
                (flow_node(plain_scalar(string_scalar))) @value
                (block_node(block_sequence(block_sequence_item(flow_node(plain_scalar(string_scalar) @item)))))
            ]
            (#eq? @key2 "extends")
        )
        "#;

        let tree = parser.parse(content, None).unwrap();
        let root_node = tree.root_node();

        let query = Query::new(language(), query_source).unwrap();
        let mut cursor_qry = QueryCursor::new();
        let matches = cursor_qry.matches(&query, root_node, content.as_bytes());

        let mut extends: Vec<GitlabExtend> = vec![];

        let valid_indexes: Vec<u32> = vec![1, 2];
        for mat in matches.into_iter() {
            for c in mat.captures {
                if valid_indexes.contains(&c.index) {
                    let text = &content[c.node.byte_range()];
                    if c.node.start_position().row != c.node.end_position().row {
                        // sanity check
                        error!(
                            "extends spans over multiple rows: uri: {} text: {}",
                            uri, text
                        );

                        continue;
                    }

                    extends.push(GitlabExtend {
                        key: text.to_owned(),
                        uri: uri.clone(),
                        range: Range {
                            start: LSPPosition {
                                line: c.node.start_position().row as u32,
                                character: c.node.start_position().column as u32,
                            },
                            end: LSPPosition {
                                line: c.node.end_position().row as u32,
                                character: c.node.end_position().column as u32,
                            },
                        },
                    });
                }
            }
        }

        extends
    }
}

impl Parser {
    pub fn new(
        remote_urls: Vec<String>,
        package_map: HashMap<String, String>,
        cache_path: String,
    ) -> Parser {
        Parser {
            remote_urls,
            package_map,
            cache_path,
        }
    }

    pub fn parse_contents(
        &self,
        uri: &Url,
        content: &str,
        _follow: bool,
    ) -> Option<(Vec<GitlabFile>, Vec<GitlabRootNode>)> {
        let mut files: Vec<GitlabFile> = vec![];
        let mut nodes: Vec<GitlabRootNode> = vec![];

        self.parse_contents_recursive(&mut files, &mut nodes, uri, content, _follow, 0)?;

        Some((files, nodes))
    }

    fn parse_contents_recursive(
        &self,
        files: &mut Vec<GitlabFile>,
        nodes: &mut Vec<GitlabRootNode>,
        uri: &lsp_types::Url,
        content: &str,
        _follow: bool,
        iteration: i32,
    ) -> Option<()> {
        // #safety wow amazed
        if iteration > 10 {
            return None;
        }

        files.push(GitlabFile {
            path: uri.as_str().into(),
            content: content.into(),
        });

        nodes.append(&mut ParserUtils::get_all_root_nodes(content));

        let (_, value) = ParserUtils::get_root_node(content, "include")?;

        if let Yaml::Array(includes) = value {
            for include in includes {
                if let Yaml::Hash(item) = include {
                    let mut remote_pkg = String::new();
                    let mut remote_tag = String::new();
                    let mut remote_files: Vec<String> = vec![];

                    for (key, item_value) in item {
                        if !remote_pkg.is_empty() {
                            self.fetch_remote_files(
                                files,
                                nodes,
                                &remote_pkg,
                                &remote_tag,
                                &remote_files,
                            );
                        }

                        if let Yaml::String(key_str) = key {
                            match key_str.trim().to_lowercase().as_str() {
                                "local" => {
                                    if let Yaml::String(value) = item_value {
                                        let current_uri = uri.join(value.as_str()).ok()?;
                                        let current_content =
                                            std::fs::read_to_string(current_uri.path()).ok()?;

                                        if _follow {
                                            self.parse_contents_recursive(
                                                files,
                                                nodes,
                                                &current_uri,
                                                &current_content,
                                                _follow,
                                                iteration + 1,
                                            );
                                        }
                                    }
                                }
                                "project" => {
                                    if let Yaml::String(value) = item_value {
                                        remote_pkg = value.clone();
                                    }
                                }
                                "ref" => {
                                    if let Yaml::String(value) = item_value {
                                        remote_tag = value.clone();
                                    }
                                }
                                "file" => {
                                    debug!("files: {:?}", item_value);
                                    if let Yaml::Array(value) = item_value {
                                        for yml in value {
                                            if let Yaml::String(_path) = yml {
                                                remote_files.push(_path);
                                            }
                                        }
                                    }
                                }
                                _ => break,
                            }
                        }
                    }

                    if !remote_pkg.is_empty() {
                        self.fetch_remote_files(
                            files,
                            nodes,
                            &remote_pkg,
                            &remote_tag,
                            &remote_files,
                        );
                    }
                }
            }
        }

        Some(())
    }

    fn fetch_remote_files(
        &self,
        files: &mut Vec<GitlabFile>,
        nodes: &mut Vec<GitlabRootNode>,
        remote_pkg: &String,
        remote_tag: &String,
        remote_files: &Vec<String>,
    ) {
        if remote_tag.is_empty() || remote_pkg.is_empty() || remote_files.is_empty() {
            return;
        }

        if let Err(err) = std::fs::create_dir_all(&self.cache_path) {
            error!("error creating cache folder; got err {}", err);

            return;
        }

        // check if we have that reference to repository
        let repo_dest = format!("{}{}/{}", &self.cache_path, remote_pkg, remote_tag);

        self.clone_repo(repo_dest.as_str(), remote_tag.as_str(), remote_pkg.as_str());

        for file in remote_files {
            let file_path = format!("{}{}", repo_dest, file);
            debug!("filepath{}", file_path);

            let content = match std::fs::read_to_string(&file_path) {
                Ok(content) => content,
                Err(err) => {
                    error!("error reading content from: {}; got err {}", file_path, err);
                    continue;
                }
            };

            let uri = match Url::parse(format!("file://{}", &file_path).as_str()) {
                Ok(uri) => uri,
                Err(err) => {
                    error!("error generating uri; got err {}", err);
                    continue;
                }
            };

            nodes.append(&mut ParserUtils::get_all_root_nodes(content.as_str()));

            files.push(GitlabFile {
                path: uri.as_str().into(),
                content,
            });
        }
    }

    fn clone_repo(&self, repo_dest: &str, remote_tag: &str, remote_pkg: &str) {
        let repo_dest_path = std::path::Path::new(repo_dest);

        info!("repo_path: {:?}", repo_dest_path);

        if repo_dest_path.exists() {
            let mut repo_contents = match repo_dest_path.read_dir() {
                Ok(contents) => contents,
                Err(err) => {
                    error!("error reading repo contents; got err: {}", err);
                    return;
                }
            };

            if repo_contents.next().is_some() {
                info!("repo contents exist");

                return;
            }

            return;
        }

        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(|_url, username_from_url, _allowed_paths| {
            Cred::ssh_key_from_agent(username_from_url.unwrap())
        });

        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(callbacks);

        debug!("remote tag {}", remote_tag);

        // Prepare builder.
        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fo);

        // Clone the project.
        let remotes = match self.package_map.get(remote_pkg) {
            Some(host) => vec![host.to_string()],
            None => self.remote_urls.clone(),
        };

        info!("got git host: {:?}", remotes);

        let dest = Path::new(repo_dest);
        info!("clone dest {:?}", dest);

        env::set_var("GIT_HTTP_LOW_SPEED_LIMIT", "1000");
        env::set_var("GIT_HTTP_LOW_SPEED_TIME", "10");

        for origin in remotes {
            info!("origin: {:?}", origin);
            match builder.clone(format!("{}:{}", origin, remote_pkg).as_str(), dest) {
                Ok(repo) => {
                    info!("repository successfully cloned: {:?}", repo.path());
                    let (object, reference) =
                        repo.revparse_ext(remote_tag).expect("Object not found");

                    repo.checkout_tree(&object, None)
                        .expect("Failed to checkout");

                    match reference {
                        Some(gref) => repo.set_head(gref.name().unwrap()),
                        None => repo.set_head_detached(object.id()),
                    }
                    .expect("Failed to set HEAD");

                    break;
                }
                Err(err) => {
                    info!("error cloning repo: {:?}", err);
                    if dest.exists() {
                        fs::remove_dir_all(dest).expect("should be able to remove");
                    }

                    continue;
                }
            }
        }
    }
}
