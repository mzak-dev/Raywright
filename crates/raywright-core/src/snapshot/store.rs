use std::path::{Path, PathBuf};

use crate::config::{Config, UpdateSnapshots};

/// Where expected pictures live and where results of a failed comparison go.
#[derive(Clone, Debug)]
pub struct SnapshotStore {
    /// Expected pictures: `<root>/<test>/<name>.png`.
    pub root: PathBuf,
    /// Actual and diff pictures: `<output>/<test>/<name>-actual.png`.
    pub output: PathBuf,
    pub update: UpdateSnapshots,
    pub ci: bool,
}

/// A test or picture name made safe as one path segment.
pub fn sanitize(s: &str) -> String {
    let out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' }).collect();
    let out = out.replace("--", "-");
    out.trim_matches('-').to_string()
}

fn with_png(name: &str) -> String {
    if name.to_ascii_lowercase().ends_with(".png") { name.to_string() } else { format!("{name}.png") }
}

impl SnapshotStore {
    pub fn from_config(c: &Config) -> Self {
        SnapshotStore { root: c.snapshot_dir(), output: c.output_dir(), update: c.snapshot.update, ci: Config::in_ci() }
    }

    pub fn expected(&self, test: &str, name: &str) -> PathBuf {
        self.root.join(sanitize(test)).join(with_png(name))
    }

    /// `(actual, diff)`.
    pub fn results(&self, test: &str, name: &str) -> (PathBuf, PathBuf) {
        let stem = with_png(name);
        let stem = stem.trim_end_matches(".png");
        let dir = self.output.join(sanitize(test));
        (dir.join(format!("{stem}-actual.png")), dir.join(format!("{stem}-diff.png")))
    }

    /// May a missing expected picture be written now?
    pub fn may_write_missing(&self) -> bool {
        match self.update {
            UpdateSnapshots::All => true,
            UpdateSnapshots::Missing => !self.ci,
            UpdateSnapshots::None => false,
        }
    }

    pub fn may_overwrite(&self) -> bool {
        self.update == UpdateSnapshots::All
    }

    pub fn relative<'a>(&self, p: &'a Path) -> &'a Path {
        p.strip_prefix(std::env::current_dir().unwrap_or_default()).unwrap_or(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        let s = SnapshotStore { root: "shots".into(), output: "out".into(), update: UpdateSnapshots::Missing, ci: true };
        assert_eq!(s.expected("e2e::widgets::clock", "clock"), PathBuf::from("shots/e2e-widgets-clock/clock.png"));
        assert_eq!(s.results("t", "a.png").0, PathBuf::from("out/t/a-actual.png"));
        assert!(!s.may_write_missing());
        assert_eq!(sanitize("a b/c::d"), "a-b-c-d");
    }
}
