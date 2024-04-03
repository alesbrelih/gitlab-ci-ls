use std::{collections::HashMap, fs, path, process::Command};

use log::{debug, error, info};
use lsp_types::Url;

use crate::GitlabFile;

pub trait Git {
    fn clone_repo(&self, repo_dest: &str, remote_tag: &str, remote_pkg: &str);
    fn fetch_remote_files(
        &self,
        remote_pkg: &str,
        remote_tag: &str,
        remote_files: &[String],
    ) -> anyhow::Result<Vec<GitlabFile>>;
}

pub struct GitImpl {
    package_map: HashMap<String, String>,
    remote_urls: Vec<String>,
    cache_path: String,
}

impl GitImpl {
    pub fn new(
        remote_urls: Vec<String>,
        package_map: HashMap<String, String>,
        cache_path: String,
    ) -> Self {
        Self {
            remote_urls,
            package_map,
            cache_path,
        }
    }
}

impl Git for GitImpl {
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

        let remotes = match self.package_map.get(remote_pkg) {
            Some(host) => vec![host.to_string()],
            None => self.remote_urls.clone(),
        };

        info!("got git host: {:?}", remotes);

        for origin in remotes {
            match Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    "--branch",
                    remote_tag,
                    format!("{}{}", origin, remote_pkg).as_str(),
                    repo_dest,
                ])
                .output()
            {
                Ok(ok) => {
                    info!("successfully cloned to : {}; got: {:?}", repo_dest, ok);
                    break;
                }
                Err(err) => {
                    error!("error cloning to: {}, got: {:?}", repo_dest, err);

                    let dest = path::Path::new(repo_dest);
                    if dest.exists() {
                        fs::remove_dir_all(dest).expect("should be able to remove");
                    }
                    continue;
                }
            };
        }
    }

    fn fetch_remote_files(
        &self,
        remote_pkg: &str,
        remote_tag: &str,
        remote_files: &[String],
    ) -> anyhow::Result<Vec<GitlabFile>> {
        if remote_tag.is_empty() || remote_pkg.is_empty() || remote_files.is_empty() {
            return Ok(vec![]);
        }

        std::fs::create_dir_all(&self.cache_path)?;

        // check if we have that reference to repository
        let repo_dest = format!("{}{}/{}", &self.cache_path, remote_pkg, remote_tag);

        self.clone_repo(repo_dest.as_str(), remote_tag, remote_pkg);

        let mut files = vec![];
        for file in remote_files {
            let file_path = format!("{}{}", repo_dest, file);
            debug!("filepath: {}", file_path);

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

            files.push(GitlabFile {
                path: uri.to_string(),
                content,
            });
        }

        Ok(files)
    }
}
