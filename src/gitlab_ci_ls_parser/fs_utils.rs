pub trait FSUtils {
    fn create_dir_all(&self, path: &str) -> anyhow::Result<()>;
}

pub struct FSUtilsImpl {
    pub home_path: String,
}

impl FSUtilsImpl {
    pub fn new(home_path: String) -> Self {
        Self { home_path }
    }

    pub fn get_path(&self, uri: &str) -> std::path::PathBuf {
        if !uri.starts_with('~') {
            return uri.to_string().into();
        }

        uri.replace('~', &self.home_path).into()
    }
}

impl FSUtils for FSUtilsImpl {
    fn create_dir_all(&self, path: &str) -> anyhow::Result<()> {
        let path = self.get_path(path);
        std::fs::create_dir_all(path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_replace_with_home() {
        let mock_home = "/my/home".to_string();
        let fsutils = FSUtilsImpl::new(mock_home.clone());

        assert_eq!(
            fsutils.get_path("~/somewhere/here").to_str().unwrap(),
            format!("{mock_home}/somewhere/here")
        );
    }
}
