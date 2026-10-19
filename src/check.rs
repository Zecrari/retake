//! `retake check`: validates cassettes and looks for leaked secrets.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::cassette::Cassette;
use crate::redact;

/// Returns true when every cassette is valid and free of secrets.
pub fn check(paths: &[PathBuf], fix: bool) -> anyhow::Result<bool> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            collect_json(path, &mut files)?;
        } else if path.is_file() {
            files.push(path.clone());
        } else {
            anyhow::bail!("{} does not exist", path.display());
        }
    }
    files.sort();

    let mut problems = 0;
    for file in &files {
        let text = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        if let Err(e) = serde_json::from_str::<Cassette>(&text) {
            println!("{}: not a valid cassette: {e}", file.display());
            problems += 1;
            continue;
        }
        let findings = redact::scan(&text);
        if findings.is_empty() {
            continue;
        }
        if fix {
            fs::write(file, redact::redact_str(&text).as_ref())
                .with_context(|| format!("writing {}", file.display()))?;
            println!("{}: redacted {} line(s)", file.display(), findings.len());
        } else {
            for finding in findings {
                println!("{}:{}: possible {}", file.display(), finding.line, finding.rule);
            }
            problems += 1;
        }
    }

    match (files.len(), problems) {
        (0, _) => println!("No cassettes found."),
        (n, 0) => println!("Checked {n} cassette(s): all clean."),
        (n, p) => {
            println!("Checked {n} cassette(s): {p} with problems.");
            if !fix {
                println!("Run `retake check --fix` to redact detected secrets in place.");
            }
        }
    }
    Ok(problems == 0)
}

fn collect_json(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_json(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_and_fixes_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let clean = Cassette::new("clean");
        clean.save(&dir.path().join("clean.json")).unwrap();
        let leaky = r#"{"retake":1,"session":"leaky","interactions":[],"note":"sk-abcdefghijklmnopqrstuvwxyz"}"#;
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("leaky.json"), leaky).unwrap();

        assert!(!check(&[dir.path().to_path_buf()], false).unwrap());
        assert!(check(&[dir.path().to_path_buf()], true).is_ok());
        assert!(check(&[dir.path().to_path_buf()], false).unwrap());
    }

    #[test]
    fn rejects_invalid_cassettes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("bad.json"), "{}").unwrap();
        assert!(!check(&[dir.path().to_path_buf()], false).unwrap());
    }
}
