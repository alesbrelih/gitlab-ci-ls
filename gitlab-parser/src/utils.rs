use std::{collections::HashMap, path::Path, sync::MutexGuard};

use git2::{Cred, RemoteCallbacks};
use log::{debug, error};
use lsp_types::Url;
use yaml_rust::{Yaml, YamlLoader};

use crate::{LSPPosition, Range};

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

fn fetch_remote_files(
    store: &mut MutexGuard<'_, HashMap<String, String>>,
    package_map: &HashMap<String, String>,
    cache_path: &str,
    remote_pkg: &String,
    remote_tag: &String,
    remote_files: &Vec<String>,
) {
    if remote_tag.is_empty() || remote_pkg.is_empty() || remote_files.is_empty() {
        return;
    }

    if let Err(err) = std::fs::create_dir_all(cache_path) {
        error!("error creating cache folder; got err {}", err);

        return;
    }

    // check if we have that reference to repository
    let repo_path = format!("{}{}/{}/", &cache_path, remote_pkg, remote_tag);
    error!("repo_path: {}", repo_path);

    if !std::path::Path::new(&repo_path).exists() {
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
        let host = match package_map.get(remote_pkg) {
            Some(host) => host,
            None => {
                return;
            }
        };

        error!("got host: {}", host);

        let dest = Path::new(repo_path.as_str());
        error!("dest {:?}", dest);

        match builder.clone(format!("{}:{}", host, remote_pkg).as_str(), dest) {
            Ok(repo) => {
                debug!("repository successfully cloned: {:?}", repo.path());
                let (object, reference) = repo.revparse_ext(remote_tag).expect("Object not found");

                repo.checkout_tree(&object, None)
                    .expect("Failed to checkout");

                match reference {
                    Some(gref) => repo.set_head(gref.name().unwrap()),
                    None => repo.set_head_detached(object.id()),
                }
                .expect("Failed to set HEAD");
            }
            Err(err) => {
                error!("error cloning repo: {:?}", err);
            }
        }
    }

    for file in remote_files {
        let file_path = format!("{}{}", repo_path, file);
        debug!("filepath{}", file_path);

        let content = match std::fs::read_to_string(&file_path) {
            Ok(content) => content,
            Err(err) => {
                error!("error reading content from: {}; got err {}", file_path, err);
                continue;
            }
        };

        let uri = match Url::parse(format!("file:/{}", &file_path).as_str()) {
            Ok(uri) => uri,
            Err(err) => {
                error!("error generating uri; got err {}", err);
                continue;
            }
        };

        store.insert(uri.as_str().into(), content);
    }
}
pub fn parse_contents(
    store: &mut MutexGuard<'_, HashMap<String, String>>,
    package_map: &HashMap<String, String>,
    cache_path: &str,
    uri: &Url,
    content: &str,
    _follow: bool,
    iteration: u16,
) -> Option<()> {
    // #safety wow amazed
    if iteration > 10 {
        return None;
    }

    store.insert(uri.as_str().into(), content.into());

    let (_, value) = get_root_node(content, "include")?;

    if let Yaml::Array(includes) = value {
        for include in includes {
            if let Yaml::Hash(item) = include {
                let mut remote_pkg = String::new();
                let mut remote_tag = String::new();
                let mut remote_files: Vec<String> = vec![];

                for (key, item_value) in item {
                    if !remote_pkg.is_empty() {
                        fetch_remote_files(
                            store,
                            package_map,
                            cache_path,
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
                                        parse_contents(
                                            store,
                                            package_map,
                                            cache_path,
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
                    fetch_remote_files(
                        store,
                        package_map,
                        cache_path,
                        &remote_pkg,
                        &remote_tag,
                        &remote_files,
                    );
                }
            }
        }
    }

    None
}
