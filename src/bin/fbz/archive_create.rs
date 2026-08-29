use std::{
    env,
    path::{Component, Path, PathBuf},
};

use fbz::{Error, Result};

fn invalid(message: impl Into<String>) -> Error { Error::InvalidConfiguration(message.into()) }

pub(super) fn archive_name(path: &Path) -> Result<PathBuf> {
    let relative = if path.is_absolute() {
        let current = env::current_dir()?;
        path.strip_prefix(&current).ok().map(Path::to_path_buf).or_else(|| path.file_name().map(PathBuf::from))
    } else { Some(path.to_path_buf()) }
    .ok_or_else(|| invalid(format!("cannot derive an archive name for {}", path.display())))?;
    let mut clean = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => clean.push(name),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(invalid(format!("refusing unsafe archive input name {}", relative.display())));
            }
        }
    }
    if clean.as_os_str().is_empty() { return Err(invalid(format!("cannot derive an archive name for {}", path.display()))); }
    Ok(clean)
}
