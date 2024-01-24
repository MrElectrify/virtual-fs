use crate::FileSystem;
use normalize_path::NormalizePath;
use path_slash::PathBufExt;
use std::io;
use std::io::ErrorKind;
use std::iter::once;
use std::path::{Component, Path, PathBuf};

/// Iterates over all path components.
///
/// # Arguments
/// `path`: The current path.  
///
/// # Example
/// ```
/// use std::path::Path;
/// use virtual_fs::util::component_iter;
///
/// itertools::assert_equal(
///     component_iter(Path::new("../many/files/and/directories/")),
///     vec!["many", "files", "and", "directories"],
/// );
/// ```
pub fn component_iter(path: &Path) -> impl DoubleEndedIterator<Item = &str> {
    path.components().filter_map(|component| {
        if let Component::Normal(component) = component {
            component.to_str()
        } else {
            None
        }
    })
}

/// Creates all directories by iteratively creating parent directories. Returns an error if the operation fails for
/// any reason other than `AlreadyExists`.
///
/// # Arguments
/// `fs`: The filesystem.  
/// `path`: The path of the directory to create.  
pub fn create_dir_all<FS: FileSystem + ?Sized>(fs: &FS, path: &str) -> crate::Result<()> {
    let normalized = normalize_path(make_relative(path));

    for path in parent_iter(&normalized).chain(once(normalized.as_ref())) {
        // unwrap: `path` should already be a valid UTF-8 string
        if let Err(err) = fs.create_dir(path.to_str().unwrap()) {
            if err.kind() != ErrorKind::AlreadyExists {
                return Err(err);
            }
        }
    }

    Ok(())
}

/// Normalizes a path by stripping slashes, resolving backtracking, and using forward slashes.
///
/// # Arguments
/// `path`: The path to normalize.  
///
/// # Example
/// ```
/// use std::path::Path;
/// use virtual_fs::util::normalize_path;
///
/// assert_eq!(normalize_path("///////"), Path::new("/"));
/// assert_eq!(normalize_path("./test/something/../"), Path::new("test"));
/// assert_eq!(normalize_path("../test"), Path::new("test"));
/// ```
pub fn normalize_path<P: AsRef<Path>>(path: P) -> PathBuf {
    Path::new(path.as_ref().normalize().to_slash_lossy().as_ref()).to_owned()
}

/// Produces an iterator iterating over all parent directories, exclusive of `path`.
///
/// # Arguments
/// `path`: The current path.  
///
/// # Example
/// ```
/// use std::path::Path;
/// use virtual_fs::util::parent_iter;
///
/// itertools::assert_equal(
///     parent_iter(Path::new("/many/files/and/directories")),
///     vec![
///         Path::new("/many/files/and"),
///         Path::new("/many/files"),
///         Path::new("/many"),
///         Path::new("/"),
///     ],
/// );
///
/// itertools::assert_equal(
///     parent_iter(Path::new("../many/files/and/directories")),
///     vec![
///         Path::new("../many/files/and"),
///         Path::new("../many/files"),
///         Path::new("../many"),
///         Path::new(".."),
///     ],
/// );
/// ```
pub fn parent_iter(path: &Path) -> impl DoubleEndedIterator<Item = &Path> {
    // collect parent paths
    path.ancestors()
        .filter(|path| !path.as_os_str().is_empty())
        .skip(1)
        .collect::<Vec<_>>()
        .into_iter()
}

/// Trims the `/` and `\\` roots off of the beginning path, making it relative.
pub(crate) fn make_relative<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref().to_str().unwrap_or("");
    path.trim_start_matches('/').trim_start_matches('\\').into()
}

/// Returns an error indicating that the path already exists.
pub(crate) fn already_exists() -> io::Error {
    io::Error::new(ErrorKind::AlreadyExists, "Already exists")
}

/// Returns an error indicating that the path already exists.
pub(crate) fn invalid_input(error: &str) -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, error)
}

/// Returns an error indicating that the path already exists.
pub(crate) fn invalid_path() -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, "Invalid path")
}

/// Returns an error indicating that the file was not found.
pub(crate) fn not_found() -> io::Error {
    io::Error::new(ErrorKind::NotFound, "File not found")
}

/// Returns an error indicating that the operation is not supported.
pub(crate) fn not_supported() -> io::Error {
    io::Error::new(ErrorKind::Unsupported, "Not supported")
}

#[cfg(test)]
pub mod test {
    use crate::file::Metadata;
    use crate::util::{component_iter, create_dir_all, normalize_path, parent_iter};
    use crate::{FileSystem, MockFileSystem};
    use std::collections::BTreeMap;
    use std::io;
    use std::io::ErrorKind;
    use std::path::Path;

    /// Reads the directory and sorts all entries into a map.
    pub(crate) fn read_directory<F: FileSystem>(fs: &F, dir: &str) -> BTreeMap<String, Metadata> {
        fs.read_dir(dir)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.path.to_str().unwrap().to_owned(), entry.metadata)
            })
            .collect()
    }

    #[test]
    fn components() {
        itertools::assert_equal(
            component_iter(Path::new("../many/files/and/directories/")),
            vec!["many", "files", "and", "directories"],
        );
    }

    const TARGET_DIR: &str = "/some/directory/somewhere/";

    #[test]
    fn create_all_happy_case() {
        let mut mock_fs = MockFileSystem::new();

        let mut i = 0;
        mock_fs.expect_create_dir().times(3).returning(move |_| {
            i += 1;

            if i == 1 {
                Err(io::Error::new(ErrorKind::AlreadyExists, ""))
            } else {
                Ok(())
            }
        });

        assert!(create_dir_all(&mock_fs, TARGET_DIR).is_ok())
    }

    #[test]
    fn create_all_error() {
        let mut mock_fs = MockFileSystem::new();

        mock_fs
            .expect_create_dir()
            .returning(|_| Err(io::Error::new(ErrorKind::Unsupported, "")));

        assert!(create_dir_all(&mock_fs, TARGET_DIR).is_err())
    }

    #[test]
    fn normalize() {
        assert_eq!(normalize_path("///////"), Path::new("/"));
        assert_eq!(normalize_path("./test/something/../"), Path::new("test"));
        assert_eq!(normalize_path("../test"), Path::new("test"));
    }

    #[test]
    fn parent() {
        itertools::assert_equal(
            parent_iter(Path::new("/many/files/and/directories")),
            vec![
                Path::new("/many/files/and"),
                Path::new("/many/files"),
                Path::new("/many"),
                Path::new("/"),
            ],
        );

        itertools::assert_equal(
            parent_iter(Path::new("../many/files/and/directories")),
            vec![
                Path::new("../many/files/and"),
                Path::new("../many/files"),
                Path::new("../many"),
                Path::new(".."),
            ],
        );
    }
}
