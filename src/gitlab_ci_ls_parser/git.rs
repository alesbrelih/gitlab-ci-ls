use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    path::{self, Path},
    process::Command,
    sync::Mutex,
    time::{Duration, Instant},
};

use super::{
    components,
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
    fn get_project_remote_uris(&self) -> Vec<String>;
}

#[allow(clippy::module_name_repetitions)]
pub struct GitImpl {
    package_map: HashMap<String, String>,
    remote_urls: Vec<String>,
    cache_path: String,
    fs_utils: Box<dyn FSUtils>,
    // Components are resolved on every document change; remember failures for a while so
    // an unreachable component doesn't trigger network calls on every keystroke.
    failed_components: Mutex<HashMap<String, Instant>>,
}

const COMPONENT_RETRY_INTERVAL: Duration = Duration::from_secs(60);

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
            failed_components: Mutex::new(HashMap::new()),
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

    fn clone_component_repo(
        repo_dest: &str,
        component_info: &ComponentInfo,
        clone_urls: &[String],
    ) -> anyhow::Result<()> {
        let repo_dest_path = std::path::Path::new(&repo_dest);

        info!("repo_path: {}", repo_dest_path.display());

        if repo_dest_path.exists() {
            let mut repo_contents = match repo_dest_path.read_dir() {
                Ok(contents) => contents,
                Err(err) => {
                    return Err(anyhow::anyhow!(
                        "error reading repo contents; got err: {err}"
                    ));
                }
            };

            if repo_contents.next().is_some() {
                info!("repo contents exist");

                return Ok(());
            }
        }

        let mut errors = vec![];
        for clone_url in clone_urls {
            let result = if components::is_partial_version(&component_info.version) {
                GitImpl::list_remote_tags(clone_url).and_then(|tags| {
                    let Some(version) = components::resolve_version(&component_info.version, &tags)
                    else {
                        return Err(anyhow::anyhow!(
                            "no tag matches version {}",
                            component_info.version
                        ));
                    };

                    info!(
                        "resolved component version {} to {version}",
                        component_info.version
                    );
                    GitImpl::clone_ref(clone_url, &version, repo_dest)
                })
            } else if GitImpl::is_valid_commit_hash(&component_info.version) {
                GitImpl::fetch_commit(clone_url, &component_info.version, repo_dest)
            } else {
                GitImpl::clone_ref(clone_url, &component_info.version, repo_dest)
            };

            match result {
                Ok(()) => {
                    info!("successfully cloned {clone_url} to: {repo_dest}");
                    return Ok(());
                }
                Err(err) => {
                    error!("error cloning {clone_url} to: {repo_dest}; got: {err}");
                    errors.push(format!("{clone_url}: {err}"));

                    // leave an empty directory so the next attempt starts from scratch
                    if repo_dest_path.exists() {
                        let _ = fs::remove_dir_all(repo_dest_path);
                        let _ = fs::create_dir_all(repo_dest_path);
                    }
                }
            }
        }

        Err(anyhow::anyhow!(
            "could not clone component repository; tried: {}",
            errors.join("; ")
        ))
    }

    fn run_git(args: &[&str]) -> anyhow::Result<String> {
        let output = Command::new("git")
            .args(args)
            // never block the language server waiting for credentials
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "git {} failed: {}",
                args.first().unwrap_or(&""),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn list_remote_tags(clone_url: &str) -> anyhow::Result<Vec<String>> {
        let output = GitImpl::run_git(&["ls-remote", "--tags", "--refs", clone_url])?;

        Ok(components::parse_ls_remote_tags(&output))
    }

    fn clone_ref(clone_url: &str, reference: &str, repo_dest: &str) -> anyhow::Result<()> {
        GitImpl::run_git(&[
            "clone", "--depth", "1", "--branch", reference, clone_url, repo_dest,
        ])
        .map(|_| ())
    }

    fn fetch_commit(clone_url: &str, sha: &str, repo_dest: &str) -> anyhow::Result<()> {
        GitImpl::run_git(&["init", "--quiet", repo_dest])?;
        GitImpl::run_git(&["-C", repo_dest, "fetch", "--depth", "1", clone_url, sha])?;
        GitImpl::run_git(&["-C", repo_dest, "checkout", "--quiet", "FETCH_HEAD"])?;

        Ok(())
    }

    /// Possible clone URLs for the component project, most specific first.
    fn component_clone_urls(&self, component_info: &ComponentInfo) -> Vec<String> {
        let project = &component_info.project;
        let mut urls = vec![];

        if let Some(origin) = self.package_map.get(project) {
            urls.push(format!("{origin}{project}"));
        }

        // reuse the way the workspace talks to the same host (ssh port, user, https...)
        for origin in &self.remote_urls {
            if ParserUtils::host_from_remote(origin) == component_info.host {
                urls.push(format!("{origin}{project}"));
            }
        }

        urls.push(format!("git@{}:{project}", component_info.host));
        urls.push(format!("https://{}/{project}.git", component_info.host));

        let mut unique = vec![];
        for url in urls {
            if !unique.contains(&url) {
                unique.push(url);
            }
        }

        unique
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

        info!("repo_path: {}", repo_dest_path.display());

        if repo_dest_path.exists() {
            let mut repo_contents = match repo_dest_path.read_dir() {
                Ok(contents) => contents,
                Err(err) => {
                    error!("error reading repo contents; got err: {err}");
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
            Some(host) => vec![host.clone()],
            None => self.remote_urls.clone(),
        };

        info!("got git host: {remotes:?}");

        for origin in remotes {
            match Command::new("git")
                .args(
                    ["clone", "--depth", "1"]
                        .into_iter()
                        .chain(remote_tag.into_iter().flat_map(|tag| {
                            // Reference/commit can't be checked out this way
                            if GitImpl::is_valid_commit_hash(tag) {
                                vec![]
                            } else {
                                vec!["--branch", tag]
                            }
                        }))
                        .chain([&format!("{origin}{remote_pkg}"), repo_dest]),
                )
                .output()
            {
                Ok(ok) => {
                    info!("successfully cloned to : {repo_dest}; got: {ok:?}");
                    if let Some(tag) = remote_tag {
                        if GitImpl::is_valid_commit_hash(tag) {
                            match Command::new("git")
                                .args(["fetch", "--depth", "1", &origin, tag])
                                .output()
                            {
                                Ok(_) => {
                                    info!("successfully fetched referenced commit: {repo_dest} @ {tag};");

                                    info!("trying to checkout the commit");
                                    match Command::new("git")
                                        .args(["-C", repo_dest, "checkout", tag])
                                        .output()
                                    {
                                        Ok(_) => {
                                            info!(
                                                "successfully checkout repo: {repo_dest} @ {tag}"
                                            );
                                        }
                                        Err(err) => {
                                            error!("error checking out repo: {repo_dest} @ {tag}; got err: {err}");
                                            fs::remove_dir_all(repo_dest)
                                                .expect("should be able to remove");
                                        }
                                    }
                                }
                                Err(err) => {
                                    error!("error fetching referenced commit repo: {repo_dest} @ {tag}; got err: {err}");
                                    fs::remove_dir_all(repo_dest)
                                        .expect("should be able to remove");
                                }
                            }
                        }
                    }
                    break;
                }
                Err(err) => {
                    error!("error cloning to: {repo_dest}, got: {err:?}");

                    let dest = path::Path::new(repo_dest);
                    if dest.exists() {
                        fs::remove_dir_all(dest).expect("should be able to remove");
                    }
                }
            }
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
                // TODO: dirty hack, fix it when time
                let file = prepend_if_needed(file, '/');
                let file_path = format!("{repo_dest}{file}");
                debug!("filepath: {file_path}");

                let content = match std::fs::read_to_string(&file_path) {
                    Ok(content) => content,
                    Err(err) => {
                        error!("error reading content from: {file_path}; got err {err}");
                        return None;
                    }
                };

                let uri = match Url::parse(format!("file://{file_path}").as_str()) {
                    Ok(uri) => uri,
                    Err(err) => {
                        error!("error generating uri; got err {err}");
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

        if let Some(failed_at) = self.failed_components.lock().unwrap().get(&repo_dest) {
            if failed_at.elapsed() < COMPONENT_RETRY_INTERVAL {
                return Err(anyhow::anyhow!(
                    "fetching {repo_dest} failed recently; retrying in at most {}s",
                    COMPONENT_RETRY_INTERVAL.as_secs()
                ));
            }
        }

        self.fs_utils.create_dir_all(&repo_dest)?;

        let clone_urls = self.component_clone_urls(&component_info);
        let result =
            GitImpl::clone_component_repo(repo_dest.as_str(), &component_info, &clone_urls);

        let mut failed_components = self.failed_components.lock().unwrap();
        if let Err(err) = result {
            failed_components.insert(repo_dest, Instant::now());
            return Err(err);
        }
        failed_components.remove(&repo_dest);
        drop(failed_components);

        ParserUtils::get_component(&repo_dest, &component_info.component)
    }

    fn get_project_remote_uris(&self) -> Vec<String> {
        self.remote_urls.clone()
    }
}

fn prepend_if_needed(input: &str, character: char) -> String {
    if input.starts_with(character) {
        input.to_string()
    } else {
        format!("{character}{input}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const PROJECT: &str = "group/components";
    const VERSIONS: [&str; 3] = ["1.0.0", "1.2.0", "2.0.0"];

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// Component project with one commit + tag per entry of `VERSIONS`, each tag having a
    /// `templates/build.yml` that contains `# version <tag>`.
    struct ComponentRepo {
        dir: TempDir,
        // commit SHA per tag
        commits: HashMap<String, String>,
    }

    impl ComponentRepo {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let repo = dir.path().join(PROJECT);
            fs::create_dir_all(repo.join("templates")).unwrap();

            git(&repo, &["init", "--quiet"]);
            // allow fetching single commits like gitlab.com does
            git(&repo, &["config", "uploadpack.allowAnySHA1InWant", "true"]);

            let mut commits = HashMap::new();
            for version in VERSIONS {
                fs::write(
                    repo.join("templates/build.yml"),
                    format!(
                        "# version {version}\nspec:\n  inputs:\n    stage:\n      default: test\n---\nbuild:\n  script: echo {version}\n"
                    ),
                )
                .unwrap();
                git(&repo, &["add", "."]);
                git(&repo, &["commit", "--quiet", "-m", version]);
                git(&repo, &["tag", version]);
                commits.insert(version.to_string(), git(&repo, &["rev-parse", "HEAD"]));
            }

            Self { dir, commits }
        }

        /// Base URL as it would be configured in the package map.
        fn origin(&self) -> String {
            format!("file://{}/", self.dir.path().display())
        }

        fn clone_url(&self) -> String {
            format!("{}{PROJECT}", self.origin())
        }
    }

    fn component_info(version: &str) -> ComponentInfo {
        ComponentInfo {
            host: "gitlab.invalid".to_string(),
            project: PROJECT.to_string(),
            component: "build".to_string(),
            version: version.to_string(),
        }
    }

    fn checked_out_version(dest: &Path) -> String {
        let content = fs::read_to_string(dest.join("templates/build.yml")).unwrap();
        content
            .lines()
            .next()
            .unwrap()
            .trim_start_matches("# version ")
            .to_string()
    }

    fn is_empty_dir(path: &Path) -> bool {
        path.read_dir().unwrap().next().is_none()
    }

    fn git_impl(repo: &ComponentRepo, cache: &TempDir) -> GitImpl {
        GitImpl::new(
            vec![],
            HashMap::from([(PROJECT.to_string(), repo.origin())]),
            format!("{}/", cache.path().display()),
            Box::new(fs_utils::FSUtilsImpl::new(String::new())),
        )
    }

    #[test]
    fn test_clone_component_repo_resolves_versions() {
        let repo = ComponentRepo::new();
        let dest = TempDir::new().unwrap();

        for (requested, expected) in [
            ("1.0.0", "1.0.0"),
            ("1", "1.2.0"),
            ("1.0", "1.0.0"),
            ("2", "2.0.0"),
            ("~latest", "2.0.0"),
        ] {
            let repo_dest = dest.path().join(requested);
            // fetch_remote_component creates the directory before cloning
            fs::create_dir_all(&repo_dest).unwrap();

            GitImpl::clone_component_repo(
                repo_dest.to_str().unwrap(),
                &component_info(requested),
                &[repo.clone_url()],
            )
            .unwrap_or_else(|err| panic!("cloning {requested} failed: {err}"));

            assert_eq!(checked_out_version(&repo_dest), expected, "{requested}");
        }
    }

    #[test]
    fn test_clone_component_repo_by_commit_sha() {
        let repo = ComponentRepo::new();
        let dest = TempDir::new().unwrap();
        let sha = &repo.commits["1.2.0"];

        GitImpl::clone_component_repo(
            dest.path().to_str().unwrap(),
            &component_info(sha),
            &[repo.clone_url()],
        )
        .unwrap();

        assert_eq!(checked_out_version(dest.path()), "1.2.0");
    }

    #[test]
    fn test_clone_component_repo_unknown_version_leaves_empty_dir() {
        let repo = ComponentRepo::new();

        for (version, error) in [
            ("3", "no tag matches version 3"),
            ("9.9.9", "git clone failed"),
            ("main-does-not-exist", "git clone failed"),
        ] {
            let dest = TempDir::new().unwrap();

            let err = GitImpl::clone_component_repo(
                dest.path().to_str().unwrap(),
                &component_info(version),
                &[repo.clone_url()],
            )
            .unwrap_err()
            .to_string();

            assert!(err.contains(error), "{version}: {err}");
            assert!(is_empty_dir(dest.path()), "{version}");
        }
    }

    #[test]
    fn test_clone_component_repo_falls_back_to_next_url() {
        let repo = ComponentRepo::new();
        let dest = TempDir::new().unwrap();
        let missing = TempDir::new().unwrap();

        GitImpl::clone_component_repo(
            dest.path().to_str().unwrap(),
            &component_info("2.0.0"),
            &[
                format!("file://{}/{PROJECT}", missing.path().display()),
                repo.clone_url(),
            ],
        )
        .unwrap();

        assert_eq!(checked_out_version(dest.path()), "2.0.0");
    }

    #[test]
    fn test_clone_component_repo_reuses_existing_checkout() {
        let dest = TempDir::new().unwrap();
        fs::write(dest.path().join("marker"), "cached").unwrap();

        // no network/git access needed when the cache is already populated
        GitImpl::clone_component_repo(
            dest.path().to_str().unwrap(),
            &component_info("1.0.0"),
            &["file:///does/not/exist".to_string()],
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(dest.path().join("marker")).unwrap(),
            "cached"
        );
    }

    #[test]
    fn test_fetch_remote_component() {
        let repo = ComponentRepo::new();
        let cache = TempDir::new().unwrap();
        let git = git_impl(&repo, &cache);

        let element = git.fetch_remote_component(component_info("1")).unwrap();

        let expected_path = format!(
            "{}/components/{PROJECT}/1/templates/build.yml",
            cache.path().display()
        );
        assert_eq!(element.uri, format!("file://{expected_path}"));
        assert!(element
            .content
            .unwrap()
            .starts_with("# version 1.2.0\nspec:"));
    }

    #[test]
    fn test_fetch_remote_component_remembers_failures() {
        let repo = ComponentRepo::new();
        let cache = TempDir::new().unwrap();
        let git = git_impl(&repo, &cache);

        let err = git
            .fetch_remote_component(component_info("3"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("no tag matches version 3"), "{err}");

        let err = git
            .fetch_remote_component(component_info("3"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("failed recently"), "{err}");

        // other versions of the same project aren't affected
        assert!(git.fetch_remote_component(component_info("2")).is_ok());
    }

    #[test]
    fn test_component_clone_urls() {
        let git = GitImpl::new(
            vec![
                "ssh://git@gitlab.example.com:2222/".to_string(),
                "git@other.host:".to_string(),
            ],
            HashMap::new(),
            String::new(),
            Box::new(fs_utils::MockFSUtils::new()),
        );

        let urls = git.component_clone_urls(&ComponentInfo {
            host: "gitlab.example.com".to_string(),
            project: "group/components".to_string(),
            component: "build".to_string(),
            version: "1.0.0".to_string(),
        });

        assert_eq!(
            urls,
            vec![
                "ssh://git@gitlab.example.com:2222/group/components",
                "git@gitlab.example.com:group/components",
                "https://gitlab.example.com/group/components.git",
            ]
        );
    }

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
