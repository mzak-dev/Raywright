//! What an agent may do. It drives a real desktop, so the defaults are narrow.

use std::path::{Path, PathBuf};

use raywright::InputMode;

#[derive(Clone, Debug)]
pub struct Policy {
    /// Programs `app_launch` may start by path. Profiles from raywright.toml are always allowed.
    pub allow_exe: Vec<PathBuf>,
    /// May `app_attach` take over apps that are already running?
    pub allow_attach: bool,
    /// Message input leaves the user's mouse alone; OS input must be asked for.
    pub input_mode: InputMode,
    /// Pictures sent back are at most this many px on their longer side.
    pub max_image_px: u32,
    /// The only place files are written.
    pub output_dir: PathBuf,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { allow_exe: Vec::new(), allow_attach: false, input_mode: InputMode::Message, max_image_px: 1568, output_dir: "raywright-mcp-output".into() }
    }
}

fn same(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b || a.file_name().is_some() && a.file_name() == b.file_name() && b.components().count() == 1,
    }
}

impl Policy {
    pub fn may_launch(&self, exe: &Path) -> Result<(), String> {
        if self.allow_exe.iter().any(|a| same(a, exe)) { Ok(()) } else { Err(format!("{} is not allowed; start the server with --allow-exe {} or use a profile", exe.display(), exe.display())) }
    }

    pub fn may_attach(&self) -> Result<(), String> {
        if self.allow_attach { Ok(()) } else { Err("attaching to running apps is off; start the server with --allow-attach".into()) }
    }

    /// A path inside the output directory (no escaping it with `..`).
    pub fn output(&self, name: &str) -> Result<PathBuf, String> {
        let p = Path::new(name);
        // `/x`, `\x` and `C:x` are not absolute on Windows yet still leave the directory in `join`,
        // and `a:b` names a stream there: no `:` anywhere
        if p.is_absolute()
            || p.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(_)))
            || name.starts_with(['/', '\\'])
            || name.contains(':')
        {
            return Err(format!("{name}: files go in the output directory, give a plain name"));
        }
        Ok(self.output_dir.join(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launching_and_files_are_limited() {
        let p = Policy { allow_exe: vec!["demo-app".into()], ..Default::default() };
        assert!(p.may_launch(Path::new("demo-app")).is_ok());
        assert!(p.may_launch(Path::new("cmd.exe")).is_err());
        assert!(p.may_attach().is_err());
        assert!(p.output("../x.png").is_err());
        assert!(p.output("/etc/x").is_err());
        assert!(p.output("\\x").is_err() && p.output("C:x").is_err() && p.output("C:\\x").is_err());
        assert_eq!(p.output("a.png").unwrap(), Path::new("raywright-mcp-output").join("a.png"));
    }
}
