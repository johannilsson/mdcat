use anyhow::{Context, Result};
use std::io::Read;
use std::path::PathBuf;

pub struct Source {
    pub name: String,
    pub content: String,
    /// Directory of the source file, for resolving relative image paths.
    pub base_dir: Option<PathBuf>,
    /// The file it was read from; `None` for stdin.
    #[cfg_attr(not(feature = "annotations"), allow(dead_code))]
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

#[cfg(test)]
mod tests {
    use super::*;

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
