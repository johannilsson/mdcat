//! Change detection for the pager: while paging, the markdown files (and, with
//! `--annotations`, their Folio sidecars) are polled so edits show up live.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::input::Source;

/// How often the pagers check for changes while idle.
pub const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// What identifies a version of a file. Folio replaces the sidecar by
/// renaming a temp file over it, so mtime and length are enough.
type Stamp = Option<(SystemTime, u64)>;

pub struct Watcher {
    files: Vec<(PathBuf, Stamp)>,
}

impl Watcher {
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let files = paths
            .into_iter()
            .map(|p| {
                let stamp = stamp(&p);
                (p, stamp)
            })
            .collect();
        Self { files }
    }

    /// Watches every file source, and with `sidecars` its Folio sidecar too
    /// (which may not exist yet). `None` when there is nothing to watch,
    /// e.g. only stdin.
    pub fn for_sources(sources: &[Source], sidecars: bool) -> Option<Self> {
        let paths: Vec<PathBuf> = sources
            .iter()
            .filter_map(|s| s.path.as_deref())
            .flat_map(|p| [Some(p.to_path_buf()), sidecars.then(|| sidecar_path(p))])
            .flatten()
            .collect();
        (!paths.is_empty()).then(|| Self::new(paths))
    }

    /// Whether any watched file changed since the last call.
    pub fn changed(&mut self) -> bool {
        let mut changed = false;
        for (path, old) in &mut self.files {
            let new = stamp(path);
            if new != *old {
                *old = new;
                changed = true;
            }
        }
        changed
    }
}

fn stamp(path: &Path) -> Stamp {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

#[cfg(feature = "annotations")]
fn sidecar_path(file: &Path) -> PathBuf {
    folio::location::folio_path(file)
}

#[cfg(not(feature = "annotations"))]
fn sidecar_path(file: &Path) -> PathBuf {
    // Never asked for: `--annotations` doesn't exist without the feature.
    file.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_files_report_no_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        fs::write(&path, "one").unwrap();

        let mut w = Watcher::new([path]);
        assert!(!w.changed());
        assert!(!w.changed());
    }

    #[test]
    fn a_rewritten_file_is_reported_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        fs::write(&path, "one").unwrap();

        let mut w = Watcher::new([path.clone()]);
        // A different length is enough even when the mtime granularity is coarse.
        fs::write(&path, "one two").unwrap();
        assert!(w.changed());
        assert!(!w.changed());
    }

    #[test]
    fn a_file_appearing_or_disappearing_is_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md.folio");

        let mut w = Watcher::new([path.clone()]);
        assert!(!w.changed());
        fs::write(&path, "{}").unwrap();
        assert!(w.changed());
        fs::remove_file(&path).unwrap();
        assert!(w.changed());
    }

    fn source(path: Option<PathBuf>) -> Source {
        Source { name: "x".into(), content: String::new(), base_dir: None, path }
    }

    #[cfg(feature = "annotations")]
    #[test]
    fn for_sources_watches_files_and_their_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        let md = dir.path().join("a.md");
        fs::write(&md, "x").unwrap();

        let mut w = Watcher::for_sources(&[source(Some(md.clone()))], true).unwrap();
        assert_eq!(w.files.len(), 2);
        assert!(!w.changed());
        // The sidecar lives under a `.folio` directory that may not exist yet.
        let sidecar = sidecar_path(&md);
        fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
        fs::write(sidecar, "{}").unwrap();
        assert!(w.changed());
    }

    #[test]
    fn for_sources_has_nothing_to_watch_for_stdin() {
        assert!(Watcher::for_sources(&[source(None)], true).is_none());
    }

    #[test]
    fn for_sources_watches_only_the_files_without_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        let md = dir.path().join("a.md");
        fs::write(&md, "x").unwrap();

        let mut w = Watcher::for_sources(&[source(Some(md.clone()))], false).unwrap();
        assert_eq!(w.files.len(), 1);
        assert!(!w.changed());
        fs::write(&md, "changed").unwrap();
        assert!(w.changed());
    }
}
