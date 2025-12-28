pub fn is_gitlab_ci_file(content: &str) -> bool {
    let yaml: serde_yaml::Value = match serde_yaml::from_str(content) {
        Ok(v) => v,
        Err(_) => return false,
    };

    if let Some(map) = yaml.as_mapping() {
        let mut has_ci_keywords = false;
        let mut has_job_definitions = false;

        for (key, value) in map {
            if let Some(key_str) = key.as_str() {
                if matches!(key_str, "stages" | "workflow" | "default") {
                    has_ci_keywords = true;
                }

                if key_str == "include" {
                    has_ci_keywords = true;
                }

                if let Some(job_map) = value.as_mapping() {
                    for job_key in job_map.keys() {
                        if let Some(jk_str) = job_key.as_str() {
                            if matches!(
                                jk_str,
                                "script" | "extends" | "before_script" | "after_script" | "rules"
                            ) {
                                has_job_definitions = true;
                            }
                        }
                    }
                }
            }
        }

        return has_ci_keywords || has_job_definitions;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_gitlab_ci_file_basic() {
        let content = r#"
stages:
  - build
  - test

job1:
  stage: build
  script:
    - echo "hello"
"#;
        assert!(is_gitlab_ci_file(content));
    }

    #[test]
    fn test_is_gitlab_ci_file_no_stages() {
        let content = r#"
job1:
  script:
    - echo "hello"
"#;
        assert!(is_gitlab_ci_file(content));
    }

    #[test]
    fn test_is_gitlab_ci_file_include() {
        let content = r#"
include:
  - local: 'configs/base.yml'
"#;
        assert!(is_gitlab_ci_file(content));
    }

    #[test]
    fn test_is_gitlab_ci_file_ansible() {
        let content = r#"
- name: Update all packages
  yum:
    name: '*'
    state: latest
"#;
        assert!(!is_gitlab_ci_file(content));
    }

    #[test]
    fn test_is_gitlab_ci_file_ansible_playbook() {
        let content = r#"
hosts: all
tasks:
  - name: test
    debug:
      msg: "hello"
"#;
        assert!(!is_gitlab_ci_file(content));
    }

    #[test]
    fn test_is_gitlab_ci_file_k8s() {
        let content = r#"
apiVersion: v1
kind: Pod
metadata:
  name: nginx
spec:
  containers:
  - name: nginx
    image: nginx:1.14.2
"#;
        assert!(!is_gitlab_ci_file(content));
    }

    #[test]
    fn test_is_gitlab_ci_file_invalid_yaml() {
        assert!(!is_gitlab_ci_file("not a yaml"));
    }
}
