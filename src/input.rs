use anyhow::{Context, Result};
use std::io::Read;
use std::path::PathBuf;

pub struct Source {
    pub name: String,
    pub content: String,
    /// Directory of the source file, for resolving relative image paths.
    pub base_dir: Option<PathBuf>,
    /// The file it was read from; `None` for stdin.
    pub path: Option<PathBuf>,
}

/// Collect all input sources from the given file paths.
/// If `files` is empty (or contains "-"), reads from stdin.
pub fn collect(files: Vec<PathBuf>) -> Result<Vec<Source>> {
    if files.is_empty() || files == vec![PathBuf::from("-")] {
        let mut content = String::new();
        std::io::stdin()
            .read_to_string(&mut content)
            .context("failed to read stdin")?;
        return Ok(vec![Source {
            name: "<stdin>".to_string(),
            content,
            base_dir: std::env::current_dir().ok(),
            path: None,
        }]);
    }

    let mut sources = Vec::new();
    for path in &files {
        if path == &PathBuf::from("-") {
            let mut content = String::new();
            std::io::stdin()
                .read_to_string(&mut content)
                .context("failed to read stdin")?;
            sources.push(Source {
                name: "<stdin>".to_string(),
                content,
                base_dir: std::env::current_dir().ok(),
                path: None,
            });
        } else {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read '{}'", path.display()))?;
            let base_dir = path.parent().map(|p| p.to_path_buf());
            sources.push(Source {
                name: path.display().to_string(),
                content,
                base_dir,
                path: Some(path.clone()),
            });
        }
    }
    Ok(sources)
}

#[cfg_attr(not(feature = "annotations"), allow(dead_code))]
/// Re-read the content of every file-backed source, leaving stdin sources as
/// they are. On error nothing is changed, so a file caught mid-write keeps its
/// previous content.
pub fn reload(sources: &mut [Source]) -> Result<()> {
    let contents = sources
        .iter()
        .map(|s| match &s.path {
            Some(path) => std::fs::read_to_string(path)
                .map(Some)
                .with_context(|| format!("failed to read '{}'", path.display())),
            None => Ok(None),
        })
        .collect::<Result<Vec<_>>>()?;
    for (source, content) in sources.iter_mut().zip(contents) {
        if let Some(content) = content {
            source.content = content;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_picks_up_changed_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.md");
        std::fs::write(&path, "old").unwrap();
        let mut sources = collect(vec![path.clone()]).unwrap();

        std::fs::write(&path, "new").unwrap();
        reload(&mut sources).unwrap();
        assert_eq!(sources[0].content, "new");
    }

    #[test]
    fn reload_keeps_stdin_sources() {
        let mut sources = vec![Source {
            name: "<stdin>".into(),
            content: "piped".into(),
            base_dir: None,
            path: None,
        }];
        reload(&mut sources).unwrap();
        assert_eq!(sources[0].content, "piped");
    }

    #[test]
    fn reload_failure_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, "A").unwrap();
        std::fs::write(&b, "B").unwrap();
        let mut sources = collect(vec![a.clone(), b.clone()]).unwrap();

        std::fs::write(&a, "A2").unwrap();
        std::fs::remove_file(&b).unwrap();
        assert!(reload(&mut sources).is_err());
        assert_eq!(sources[0].content, "A");
        assert_eq!(sources[1].content, "B");
    }

    #[test]
    fn collect_reads_file_with_name_and_base_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.md");
        std::fs::write(&path, "# Hi\n").unwrap();

        let sources = collect(vec![path.clone()]).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name, path.display().to_string());
        assert_eq!(sources[0].content, "# Hi\n");
        assert_eq!(sources[0].base_dir.as_deref(), Some(dir.path()));
    }

    #[test]
    fn collect_preserves_order_of_multiple_files() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, "A").unwrap();
        std::fs::write(&b, "B").unwrap();

        let sources = collect(vec![b, a]).unwrap();
        let contents: Vec<_> = sources.iter().map(|s| s.content.as_str()).collect();
        assert_eq!(contents, ["B", "A"]);
    }

    #[test]
    fn collect_missing_file_error_names_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.md");

        let err = collect(vec![path.clone()]).err().expect("should fail");
        assert!(format!("{err:#}").contains(&path.display().to_string()));
    }

    #[test]
    fn collect_invalid_utf8_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.md");
        std::fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();

        assert!(collect(vec![path]).is_err());
    }
}
