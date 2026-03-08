use anyhow::{Context, Result};
use std::io::Read;
use std::path::PathBuf;

pub struct Source {
    pub name: String,
    pub content: String,
    /// Directory of the source file, for resolving relative image paths.
    pub base_dir: Option<PathBuf>,
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
            });
        } else {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read '{}'", path.display()))?;
            let base_dir = path.parent().map(|p| p.to_path_buf());
            sources.push(Source {
                name: path.display().to_string(),
                content,
                base_dir,
            });
        }
    }
    Ok(sources)
}
