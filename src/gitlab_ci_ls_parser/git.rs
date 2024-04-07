use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    path,
    process::Command,
    time::Duration,
};

use super::{parser_utils, GitlabFile};
use log::{debug, error, info};
use reqwest::{blocking::Client, header::IF_NONE_MATCH, StatusCode, Url};

pub trait Git {
    fn clone_repo(&self, repo_dest: &str, remote_tag: &str, remote_pkg: &str);
    fn fetch_remote_repository(
        &self,
        remote_pkg: &str,
        remote_tag: &str,
        remote_files: Vec<String>,
    ) -> anyhow::Result<Vec<GitlabFile>>;
    fn fetch_remote(&self, url: Url) -> anyhow::Result<GitlabFile>;
}

#[allow(clippy::module_name_repetitions)]
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
            package_map,
            remote_urls,
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
                    format!("{origin}{remote_pkg}").as_str(),
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

    fn fetch_remote_repository(
        &self,
        remote_pkg: &str,
        remote_tag: &str,
        remote_files: Vec<String>,
    ) -> anyhow::Result<Vec<GitlabFile>> {
        if remote_tag.is_empty() || remote_pkg.is_empty() || remote_files.is_empty() {
            return Ok(vec![]);
        }

        std::fs::create_dir_all(&self.cache_path)?;

        // check if we have that reference to repository
        let repo_dest = format!("{}{}/{}", &self.cache_path, remote_pkg, remote_tag);

        self.clone_repo(repo_dest.as_str(), remote_tag, remote_pkg);

        let files = remote_files
            .iter()
            .filter_map(|file| {
                let file_path = format!("{repo_dest}{file}");
                debug!("filepath: {}", file_path);

                let content = match std::fs::read_to_string(&file_path) {
                    Ok(content) => content,
                    Err(err) => {
                        error!("error reading content from: {}; got err {}", file_path, err);
                        return None;
                    }
                };

                let uri = match Url::parse(format!("file://{file_path}").as_str()) {
                    Ok(uri) => uri,
                    Err(err) => {
                        error!("error generating uri; got err {}", err);
                        return None;
                    }
                };

                Some(GitlabFile {
                    path: uri.to_string(),
                    content,
                })
            })
            .collect();

        Ok(files)
    }

    fn fetch_remote(&self, url: Url) -> anyhow::Result<GitlabFile> {
        let remote_cache_path = format!("{}remotes", &self.cache_path);
        std::fs::create_dir_all(&remote_cache_path)?;

        // check if file was changed
        let file_hash = parser_utils::ParserUtils::remote_path_to_hash(url.as_str());
        let file_name_pattern = format!("_{file_hash}.yaml");

        let dir_entry = fs::read_dir(&remote_cache_path)?
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(&file_name_pattern)
            });

        let (existing_name_full, file_path) = dir_entry
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path(),
                )
            })
            .unzip();

        // Extracting etag from the filename
        let existing_etag = existing_name_full
            .as_ref()
            .and_then(|name| name.split('_').next())
            .map(String::from);

        let client = Client::builder().timeout(Duration::from_secs(4)).build()?;

        let mut req = client.get(url);
        if let Some(etag) = &existing_etag {
            req = req.header(IF_NONE_MATCH, format!("\"{etag}\""));
        }

        let response = req.send()?;

        if response.status() == StatusCode::NOT_MODIFIED {
            let fpath = file_path.expect("File path must exist for NOT_MODIFIED response");
            let content = fs::read_to_string(&fpath)?;

            info!("CACHED");

            Ok(GitlabFile {
                path: format!("file://{}", fpath.to_str().unwrap()),
                content,
            })
        } else {
            info!("NOT CACHED");

            let headers = response.headers().clone();
            let etag = headers.get("etag").unwrap().to_str()?;
            let text = response.text()?;

            let path = format!(
                "{}/{}_{}.yaml",
                remote_cache_path,
                parser_utils::ParserUtils::strip_quotes(etag),
                file_hash
            );

            let mut file = File::create(&path)?;
            file.write_all(text.as_bytes())?;

            Ok(GitlabFile {
                path,
                content: text,
            })
        }
    }
}
