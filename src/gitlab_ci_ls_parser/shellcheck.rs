use std::{
    io::Write,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};

use log::{error, warn};
use lsp_types::{CodeDescription, Diagnostic, DiagnosticSeverity, NumberOrString, Position, Url};
use regex::Regex;
use serde::Deserialize;

use super::{GitlabScriptElement, GitlabScriptItem, GitlabScriptKind, ShellcheckConfig};

const MAX_PARALLEL_CHECKS: usize = 8;

pub struct Shellcheck {
    cfg: ShellcheckConfig,
    // set when the binary can't be started so it isn't retried on every save
    unavailable: AtomicBool,
}

// One line of the shell script handed to shellcheck and where it came from in
// the yaml file.
#[derive(Debug)]
struct ShellLine {
    text: String,
    row: u32,
    // utf16 length of the yaml line in front of `text`
    prefix: u32,
    // set when columns can't be mapped back (escaped quoted scalars), then the
    // whole item is reported
    whole: Option<lsp_types::Range>,
    // yaml folds this line into the next one
    continued: bool,
}

#[derive(Debug, Default)]
struct ShellDocument {
    lines: Vec<ShellLine>,
}

#[derive(Debug, Deserialize)]
struct ShellcheckOutput {
    comments: Vec<ShellcheckComment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShellcheckComment {
    line: usize,
    end_line: usize,
    column: usize,
    end_column: usize,
    level: String,
    code: u32,
    message: String,
}

impl Shellcheck {
    pub fn new(cfg: ShellcheckConfig) -> Self {
        Shellcheck {
            cfg,
            unavailable: AtomicBool::new(false),
        }
    }

    pub fn diagnostics(&self, scripts: &[GitlabScriptElement], content: &str) -> Vec<Diagnostic> {
        if !self.cfg.enabled || self.unavailable.load(Ordering::Relaxed) {
            return vec![];
        }

        let documents = get_documents(scripts, content);

        let mut diagnostics = vec![];
        for chunk in documents.chunks(MAX_PARALLEL_CHECKS) {
            std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|document| scope.spawn(move || self.check(document)))
                    .collect();

                for handle in handles {
                    diagnostics.extend(handle.join().unwrap_or_default());
                }
            });
        }

        diagnostics
    }

    fn check(&self, document: &ShellDocument) -> Vec<Diagnostic> {
        if self.unavailable.load(Ordering::Relaxed) {
            return vec![];
        }

        let mut command = Command::new(&self.cfg.path);
        command
            .arg("--format=json1")
            .arg(format!("--shell={}", self.cfg.shell))
            .arg(format!("--severity={}", self.cfg.severity));

        if !self.cfg.exclude.is_empty() {
            command.arg(format!("--exclude={}", self.cfg.exclude.join(",")));
        }

        let child = command
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(err) => {
                if !self.unavailable.swap(true, Ordering::Relaxed) {
                    warn!(
                        "shellcheck disabled; could not start '{}'; got err: {err}",
                        self.cfg.path
                    );
                }

                return vec![];
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            if let Err(err) = stdin.write_all(document.text().as_bytes()) {
                error!("error writing script to shellcheck; got err: {err}");
            }
        }

        let output = match child.wait_with_output() {
            Ok(o) => o,
            Err(err) => {
                error!("error waiting for shellcheck; got err: {err}");
                return vec![];
            }
        };

        match serde_json::from_slice::<ShellcheckOutput>(&output.stdout) {
            Ok(parsed) => parsed
                .comments
                .iter()
                .filter_map(|c| document.diagnostic(c))
                .collect(),
            Err(err) => {
                error!(
                    "error parsing shellcheck output; status: {}; got err: {err}",
                    output.status
                );
                vec![]
            }
        }
    }
}

impl ShellDocument {
    fn text(&self) -> String {
        let mut text = String::new();
        for line in &self.lines {
            text.push_str(&line.text);
            if line.continued {
                text.push_str(" \\");
            }
            text.push('\n');
        }

        text
    }

    // shellcheck lines and columns are 1 based, columns count characters
    fn position(&self, line: usize, column: usize, end: bool) -> Option<Position> {
        let shell_line = self.lines.get(line.checked_sub(1)?)?;

        if let Some(range) = shell_line.whole {
            return Some(if end { range.end } else { range.start });
        }

        let character: usize = shell_line
            .text
            .chars()
            .take(column.saturating_sub(1))
            .map(char::len_utf16)
            .sum();

        Some(Position {
            line: shell_line.row,
            character: shell_line.prefix + to_u32(character),
        })
    }

    fn diagnostic(&self, comment: &ShellcheckComment) -> Option<Diagnostic> {
        let start = self.position(comment.line, comment.column, false)?;
        let end = self
            .position(comment.end_line, comment.end_column, true)
            .filter(|end| (end.line, end.character) >= (start.line, start.character))
            .unwrap_or(start);

        let severity = match comment.level.as_str() {
            "error" => DiagnosticSeverity::ERROR,
            "warning" => DiagnosticSeverity::WARNING,
            "info" => DiagnosticSeverity::INFORMATION,
            _ => DiagnosticSeverity::HINT,
        };

        let code = format!("SC{}", comment.code);

        Some(Diagnostic {
            range: lsp_types::Range { start, end },
            severity: Some(severity),
            code_description: Url::parse(&format!("https://www.shellcheck.net/wiki/{code}"))
                .ok()
                .map(|href| CodeDescription { href }),
            code: Some(NumberOrString::String(code)),
            source: Some("shellcheck".to_string()),
            message: comment.message.clone(),
            ..Default::default()
        })
    }
}

// Gitlab runs before_script and script of a job in the same shell, after_script
// gets its own one.
fn get_documents(scripts: &[GitlabScriptElement], content: &str) -> Vec<ShellDocument> {
    let yaml_lines: Vec<&str> = content.split('\n').collect();

    let mut job_ids: Vec<usize> = vec![];
    for script in scripts {
        if !job_ids.contains(&script.job_id) {
            job_ids.push(script.job_id);
        }
    }

    let mut documents = vec![];
    for job_id in job_ids {
        for keys in [&["before_script", "script"][..], &["after_script"][..]] {
            let mut document = ShellDocument::default();

            for key in keys {
                scripts
                    .iter()
                    .filter(|s| s.job_id == job_id && s.key == *key)
                    .flat_map(|s| s.items.iter())
                    .for_each(|item| document.lines.extend(get_lines(item, &yaml_lines)));
            }

            if !document.lines.is_empty() {
                documents.push(document);
            }
        }
    }

    documents
}

fn get_lines(item: &GitlabScriptItem, yaml_lines: &[&str]) -> Vec<ShellLine> {
    let row = item.range.start.line;

    let mut lines = match item.kind {
        GitlabScriptKind::Literal | GitlabScriptKind::Folded => get_block_lines(item),
        GitlabScriptKind::Plain
        | GitlabScriptKind::SingleQuoted
        | GitlabScriptKind::DoubleQuoted => get_flow_lines(item, yaml_lines),
    };

    if item.kind != GitlabScriptKind::Literal {
        for i in 1..lines.len() {
            if lines[i].row != lines[i - 1].row
                && !lines[i - 1].text.trim().is_empty()
                && !lines[i].text.trim().is_empty()
            {
                lines[i - 1].continued = true;
            }
        }
    }

    for line in &mut lines {
        line.text = replace_gitlab_interpolation(&line.text);
        line.row += row;
    }

    lines
}

// Rows are relative to the start of the item.
fn get_block_lines(item: &GitlabScriptItem) -> Vec<ShellLine> {
    // first line is the block scalar header (|, >-, ..)
    let mut raw: Vec<&str> = item
        .content
        .split('\n')
        .skip(1)
        .map(|l| l.trim_end_matches('\r'))
        .collect();

    while raw.last().is_some_and(|l| l.trim().is_empty()) {
        raw.pop();
    }

    let indentation = |l: &str| l.len() - l.trim_start_matches(' ').len();
    let indent = raw
        .iter()
        .find(|l| !l.trim().is_empty())
        .map_or(0, |l| indentation(l));

    raw.iter()
        .enumerate()
        .map(|(i, l)| {
            let strip = indent.min(indentation(l));

            ShellLine {
                text: l[strip..].to_string(),
                row: to_u32(i + 1),
                prefix: to_u32(strip),
                whole: None,
                continued: false,
            }
        })
        .collect()
}

// Rows are relative to the start of the item.
fn get_flow_lines(item: &GitlabScriptItem, yaml_lines: &[&str]) -> Vec<ShellLine> {
    let quoted = item.kind != GitlabScriptKind::Plain;
    let inner = if quoted && item.content.len() >= 2 {
        &item.content[1..item.content.len() - 1]
    } else {
        item.content.as_str()
    };

    let escaped = match item.kind {
        GitlabScriptKind::DoubleQuoted => inner.contains('\\'),
        GitlabScriptKind::SingleQuoted => inner.contains("''"),
        _ => false,
    };

    let whole = escaped.then(|| lsp_types::Range {
        start: get_position(
            yaml_lines,
            item.range.start.line,
            item.range.start.character,
        ),
        end: get_position(yaml_lines, item.range.end.line, item.range.end.character),
    });

    let first_column = item.range.start.character + u32::from(quoted);
    let first_prefix = get_position(yaml_lines, item.range.start.line, first_column).character;

    let mut lines = vec![];
    for (i, l) in inner.split('\n').enumerate() {
        let l = l.trim_end_matches('\r');
        let (text, prefix) = if i == 0 {
            (l, first_prefix)
        } else {
            let trimmed = l.trim_start();
            (
                trimmed,
                to_u32(l[..l.len() - trimmed.len()].encode_utf16().count()),
            )
        };

        let text = match item.kind {
            GitlabScriptKind::DoubleQuoted if escaped => unescape_double_quoted(text),
            GitlabScriptKind::SingleQuoted if escaped => text.replace("''", "'"),
            _ => text.to_string(),
        };

        // escaped newlines end up as multiple shell lines of the same yaml row
        for text in text.split('\n') {
            lines.push(ShellLine {
                text: text.to_string(),
                row: to_u32(i),
                prefix,
                whole,
                continued: false,
            });
        }
    }

    lines
}

fn unescape_double_quoted(text: &str) -> String {
    let mut unescaped = String::with_capacity(text.len());
    let mut chars = text.chars();

    while let Some(c) = chars.next() {
        if c != '\\' {
            unescaped.push(c);
            continue;
        }

        match chars.next() {
            Some('n') => unescaped.push('\n'),
            Some('t') => unescaped.push('\t'),
            Some(escaped @ ('"' | '\\' | '/' | ' ')) => unescaped.push(escaped),
            Some(other) => {
                unescaped.push('\\');
                unescaped.push(other);
            }
            None => unescaped.push('\\'),
        }
    }

    unescaped
}

// $[[ inputs.name ]] is replaced by gitlab before the shell sees it. Length is
// kept so columns still match.
fn replace_gitlab_interpolation(text: &str) -> String {
    static INTERPOLATION: OnceLock<Regex> = OnceLock::new();
    let re = INTERPOLATION.get_or_init(|| Regex::new(r"\$\[\[.*?\]\]").expect("Invalid REGEX"));

    re.replace_all(text, |captures: &regex::Captures| {
        "x".repeat(captures[0].chars().count())
    })
    .to_string()
}

// Treesitter columns are bytes, lsp wants utf16.
fn get_position(yaml_lines: &[&str], row: u32, column: u32) -> Position {
    let character = yaml_lines
        .get(row as usize)
        .and_then(|l| l.get(..column as usize))
        .map_or(column, |prefix| to_u32(prefix.encode_utf16().count()));

    Position {
        line: row,
        character,
    }
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use crate::gitlab_ci_ls_parser::treesitter::{Treesitter, TreesitterImpl};

    use super::*;

    fn documents(content: &str) -> Vec<ShellDocument> {
        let scripts = TreesitterImpl::new().get_all_scripts(content);
        get_documents(&scripts, content)
    }

    fn comment(line: usize, column: usize, end_column: usize) -> ShellcheckComment {
        ShellcheckComment {
            line,
            end_line: line,
            column,
            end_column,
            level: "info".to_string(),
            code: 2086,
            message: "Double quote to prevent globbing and word splitting.".to_string(),
        }
    }

    fn range(document: &ShellDocument, comment: &ShellcheckComment) -> (u32, u32, u32, u32) {
        let range = document.diagnostic(comment).unwrap().range;
        (
            range.start.line,
            range.start.character,
            range.end.line,
            range.end.character,
        )
    }

    #[test]
    fn test_documents_group_scripts_by_job() {
        let content = r"
job_one:
  before_script:
    - x=1
  script:
    - echo $x
  after_script:
    - echo after

job_two:
  script: echo two
";

        let documents = documents(content);

        assert_eq!(3, documents.len());
        assert_eq!("x=1\necho $x\n", documents[0].text());
        assert_eq!("echo after\n", documents[1].text());
        assert_eq!("echo two\n", documents[2].text());
    }

    #[test]
    fn test_plain_and_quoted_positions() {
        let content = r#"
job:
  script:
    - echo $x
    - "echo $y"
    - 'echo $z'
"#;

        let documents = documents(content);
        let document = &documents[0];

        assert_eq!("echo $x\necho $y\necho $z\n", document.text());
        assert_eq!((3, 11, 3, 13), range(document, &comment(1, 6, 8)));
        assert_eq!((4, 12, 4, 14), range(document, &comment(2, 6, 8)));
        assert_eq!((5, 12, 5, 14), range(document, &comment(3, 6, 8)));
    }

    #[test]
    fn test_literal_block_positions() {
        let content = r"
job:
  script:
    - |
      if true; then
        echo $x
      fi

      echo done
";

        let documents = documents(content);
        let document = &documents[0];

        assert_eq!(
            "if true; then\n  echo $x\nfi\n\necho done\n",
            document.text()
        );
        assert_eq!((5, 13, 5, 15), range(document, &comment(2, 8, 10)));
        assert_eq!((8, 6, 8, 10), range(document, &comment(5, 1, 5)));
    }

    #[test]
    fn test_folded_lines_are_continued() {
        let content = r"
job:
  script:
    - >
      curl --fail
      $url

      echo done
    - curl --fail
      $other
";

        let documents = documents(content);
        let document = &documents[0];

        assert_eq!(
            "curl --fail \\\n$url\n\necho done\ncurl --fail \\\n$other\n",
            document.text()
        );
        assert_eq!((5, 6, 5, 10), range(document, &comment(2, 1, 5)));
        assert_eq!((9, 6, 9, 12), range(document, &comment(6, 1, 7)));
    }

    #[test]
    fn test_escaped_scalars_report_whole_item() {
        let content = r#"
job:
  script:
    - "echo \"$x\""
    - 'echo ''$y'''
"#;

        let documents = documents(content);
        let document = &documents[0];

        assert_eq!("echo \"$x\"\necho '$y'\n", document.text());
        assert_eq!((3, 6, 3, 19), range(document, &comment(1, 7, 9)));
        assert_eq!((4, 6, 4, 19), range(document, &comment(2, 7, 9)));
    }

    #[test]
    fn test_positions_are_utf16() {
        let content = "
job:
  script:
    - echo 😀 $x
";

        let documents = documents(content);

        assert_eq!((3, 14, 3, 16), range(&documents[0], &comment(1, 8, 10)));
    }

    #[test]
    fn test_references_and_aliases_are_skipped() {
        let content = r"
job:
  script:
    - !reference [.other, script]
    - *anchor
    - echo kept
";

        let documents = documents(content);

        assert_eq!(1, documents.len());
        assert_eq!("echo kept\n", documents[0].text());
    }

    #[test]
    fn test_gitlab_interpolation_keeps_length() {
        assert_eq!(
            "echo xxxxxxxxxxxxxxxxxx done",
            replace_gitlab_interpolation("echo $[[ inputs.name ]] done")
        );
    }

    #[test]
    fn test_diagnostic_from_shellcheck_output() {
        let output = r#"{"comments":[{"file":"-","line":1,"endLine":1,"column":6,"endColumn":8,"level":"info","code":2086,"message":"Double quote to prevent globbing and word splitting.","fix":null}]}"#;
        let parsed = serde_json::from_str::<ShellcheckOutput>(output).unwrap();

        let documents = documents("job:\n  script: echo $x\n");
        let diagnostic = documents[0].diagnostic(&parsed.comments[0]).unwrap();

        assert_eq!(Some(DiagnosticSeverity::INFORMATION), diagnostic.severity);
        assert_eq!(
            Some(NumberOrString::String("SC2086".to_string())),
            diagnostic.code
        );
        assert_eq!(Some("shellcheck".to_string()), diagnostic.source);
        assert_eq!(1, diagnostic.range.start.line);
        assert_eq!(15, diagnostic.range.start.character);
    }

    #[test]
    fn test_missing_binary_disables_shellcheck() {
        let shellcheck = Shellcheck::new(ShellcheckConfig {
            path: "/nonexistent/shellcheck".to_string(),
            ..Default::default()
        });

        let content = "job:\n  script: echo $x\n";
        let scripts = TreesitterImpl::new().get_all_scripts(content);

        assert_eq!(0, shellcheck.diagnostics(&scripts, content).len());
        assert!(shellcheck.unavailable.load(Ordering::Relaxed));
    }

    #[test]
    fn test_disabled_shellcheck_reports_nothing() {
        let shellcheck = Shellcheck::new(ShellcheckConfig {
            enabled: false,
            ..Default::default()
        });

        let content = "job:\n  script: echo $x\n";
        let scripts = TreesitterImpl::new().get_all_scripts(content);

        assert_eq!(0, shellcheck.diagnostics(&scripts, content).len());
    }

    // Needs the real binary, skipped when it isn't installed.
    #[test]
    fn test_diagnostics_with_real_shellcheck() {
        if Command::new("shellcheck")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }

        let content = r"
job:
  before_script:
    - target=$(date)
  script:
    - |
      echo deploying
      echo $target
";

        let scripts = TreesitterImpl::new().get_all_scripts(content);
        let diagnostics =
            Shellcheck::new(ShellcheckConfig::default()).diagnostics(&scripts, content);

        assert_eq!(1, diagnostics.len());
        assert_eq!(
            Some(NumberOrString::String("SC2086".to_string())),
            diagnostics[0].code
        );
        assert_eq!(7, diagnostics[0].range.start.line);
        assert_eq!(11, diagnostics[0].range.start.character);
        assert_eq!(18, diagnostics[0].range.end.character);
    }
}
