use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    path::{self, Path},
    process::Command,
    time::Duration,
};

use super::{
    fs_utils::{self, FSUtils},
    parser_utils::{self, ComponentInfo, ParserUtils},
    GitlabElement, GitlabFile, ProjectFile, DEFAULT_BRANCH_SUBFOLDER,
};
use log::{debug, error, info};
use reqwest::{blocking::Client, header::IF_NONE_MATCH, StatusCode, Url};

pub trait Git {
    fn clone_repo(&self, repo_dest: &str, remote_tag: Option<&str>, remote_pkg: &str);
    fn fetch_remote_repository(
        &self,
        remote_pkg: &str,
        remote_tag: Option<&str>,
        remote_files: ProjectFile,
    ) -> anyhow::Result<Vec<GitlabFile>>;
    fn fetch_remote(&self, url: Url) -> anyhow::Result<GitlabFile>;
    fn fetch_remote_component(
        &self,
        component_info: ComponentInfo,
    ) -> anyhow::Result<GitlabElement>;
}

#[allow(clippy::module_name_repetitions)]
pub struct GitImpl {
    package_map: HashMap<String, String>,
    remote_urls: Vec<String>,
    cache_path: String,
    fs_utils: Box<dyn FSUtils>,
}

impl GitImpl {
    pub fn new(
        remote_urls: Vec<String>,
        package_map: HashMap<String, String>,
        cache_path: String,
        fs_utils: Box<dyn fs_utils::FSUtils>,
    ) -> Self {
        Self {
            package_map,
            remote_urls,
            cache_path,
            fs_utils,
        }
    }

    fn is_valid_semver(s: &str) -> bool {
        let parts: Vec<&str> = s.split('.').collect();

        if parts.len() < 3 {
            return false;
        }

        for part in &parts {
            if !part.chars().all(|c| c.is_ascii_digit()) {
                return false;
            }
        }

        true
    }

    fn is_valid_commit_hash(s: &str) -> bool {
        let len = s.len();

        if !(7..=40).contains(&len) {
            return false;
        }

        s.chars().all(|c| c.is_ascii_hexdigit())
    }

    fn is_not_semver_or_commit_hash(s: &str) -> bool {
        !(GitImpl::is_valid_semver(s) || GitImpl::is_valid_commit_hash(s))
    }

    fn clone_component_repo(repo_dest: &str, component_info: &ComponentInfo) {
        let repo_dest_path = std::path::Path::new(&repo_dest);

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

        match Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                "--branch",
                &component_info.version,
                format!("git@{}:{}", component_info.host, component_info.project).as_str(),
                repo_dest,
            ])
            .output()
        {
            Ok(ok) => {
                info!("successfully cloned to : {}; got: {:?}", repo_dest, ok);
            }
            Err(err) => {
                error!("error cloning to: {}, got: {:?}", repo_dest, err);

                let dest = path::Path::new(repo_dest);
                if dest.exists() {
                    fs::remove_dir_all(dest).expect("should be able to remove");
                }
            }
        };
    }

    fn get_clone_repo_destination(
        cache_path: &str,
        remote_pkg: &str,
        remote_tag: Option<&str>,
    ) -> anyhow::Result<String> {
        let mut path = Path::new(cache_path).join(remote_pkg);

        // if we have a tag, add it to the path
        if let Some(tag) = remote_tag {
            path = path.join(tag);
        } else {
            path = path.join(DEFAULT_BRANCH_SUBFOLDER);
        }

        match path.to_str() {
            Some(path) => Ok(path.to_string()),
            None => Err(anyhow::anyhow!("invalid path")),
        }
    }
}

impl Git for GitImpl {
    fn clone_repo(&self, repo_dest: &str, remote_tag: Option<&str>, remote_pkg: &str) {
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

                // TODO: introduce short timeout
                if remote_tag.is_none()
                    || GitImpl::is_not_semver_or_commit_hash(remote_tag.unwrap())
                {
                    match Command::new("git").args(["-C", repo_dest, "pull"]).output() {
                        Ok(_) => info!("{repo_dest}: successfully updated using git clone"),
                        Err(err) => {
                            error!("error using git clone inside: {repo_dest}; got: {err:?}");
                        }
                    }
                } else {
                    info!("skipping git pull on {repo_dest}; ref: {remote_tag:?}; because either remote ref isn't defined or reference is a commit hash or a semver tag");
                }

                return;
            }
        }

        let remotes = match self.package_map.get(remote_pkg) {
            Some(host) => vec![host.to_string()],
            None => self.remote_urls.clone(),
        };

        info!("got git host: {:?}", remotes);

        for origin in remotes {
            // FIX: fix this because it doesn't work if reference is a commit hash
            // in this case I need to only clone without branch and then checkout the commit
            match Command::new("git")
                .args(
                    ["clone", "--depth", "1"]
                        .into_iter()
                        .chain(remote_tag.into_iter().flat_map(|tag| ["--branch", tag]))
                        .chain([&format!("{origin}{remote_pkg}"), repo_dest]),
                )
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
        remote_tag: Option<&str>,
        remote_files: ProjectFile,
    ) -> anyhow::Result<Vec<GitlabFile>> {
        let files = match remote_files {
            ProjectFile::Multi(files) => files,
            ProjectFile::Single(single) => vec![single],
        };

        if remote_pkg.is_empty() || files.is_empty() {
            return Ok(vec![]);
        }

        self.fs_utils.create_dir_all(&self.cache_path)?;

        let repo_dest =
            GitImpl::get_clone_repo_destination(&self.cache_path, remote_pkg, remote_tag)?;

        self.clone_repo(repo_dest.as_str(), remote_tag, remote_pkg);

        let files = files
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
        self.fs_utils.create_dir_all(&remote_cache_path)?;

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

    fn fetch_remote_component(
        &self,
        component_info: ComponentInfo,
    ) -> anyhow::Result<GitlabElement> {
        // TODO: handle slashes correctly..

        let repo_dest = ParserUtils::get_component_dest_dir(&self.cache_path, &component_info);
        self.fs_utils.create_dir_all(&repo_dest)?;

        GitImpl::clone_component_repo(repo_dest.as_str(), &component_info);
        ParserUtils::get_component(&repo_dest, &component_info.component)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_clone_repo_destination_no_tag() {
        let cache_path = "/home/test/.cache/gitlab-ci-ls/";
        let remote_pkg = "repo/project";
        let tag = None;

        let repo_dest = GitImpl::get_clone_repo_destination(cache_path, remote_pkg, tag);
        assert_eq!(
            repo_dest.unwrap(),
            "/home/test/.cache/gitlab-ci-ls/repo/project/default"
        );
    }

    #[test]
    fn test_get_clone_repo_destination_with_tag() {
        let cache_path = "/home/test/.cache/gitlab-ci-ls/";
        let remote_pkg = "repo/project";
        let tag = Some("1.0.0");

        let repo_dest = GitImpl::get_clone_repo_destination(cache_path, remote_pkg, tag);
        assert_eq!(
            repo_dest.unwrap(),
            "/home/test/.cache/gitlab-ci-ls/repo/project/1.0.0"
        );
    }
}
