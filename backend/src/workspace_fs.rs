use cap_std::{ambient_authority, fs::Dir};
use std::{io, path::PathBuf};

/// Run blocking filesystem work relative to a directory capability rooted at a
/// workspace. All child paths are resolved by `cap-std`, which refuses paths
/// and symbolic links that would escape the opened workspace directory.
pub async fn within_workspace<T, F>(root: PathBuf, operation: F) -> io::Result<T>
where
    T: Send + 'static,
    F: FnOnce(&Dir) -> io::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let workspace = Dir::open_ambient_dir(root, ambient_authority())?;
        operation(&workspace)
    })
    .await
    .map_err(|error| io::Error::other(format!("workspace operation task failed: {error}")))?
}
