use crate::file::{DirEntry, File, Metadata, OpenOptions};
use crate::util::{make_relative, normalize_path};
use crate::FileSystem;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

/// A filesystem that restricts access to a subdirectory of an inner filesystem.
/// Paths that would escape the scope via `../` traversal are rejected.
pub struct ScopedFS<FS: FileSystem> {
    inner: FS,
    root: PathBuf, // normalized, relative (e.g. "src")
}

impl<FS: FileSystem> ScopedFS<FS> {
    pub fn new(root: &str, inner: FS) -> Self {
        Self {
            inner,
            root: normalize_path(make_relative(root)),
        }
    }

    fn resolve(&self, path: &str) -> crate::Result<String> {
        let joined = normalize_path(self.root.join(make_relative(path)));
        if !joined.starts_with(&self.root) {
            return Err(io::Error::new(ErrorKind::PermissionDenied, "Traversal prevented"));
        }
        Ok(joined.to_str().unwrap().to_owned())
    }
}

impl<FS: FileSystem> FileSystem for ScopedFS<FS> {
    fn create_dir(&self, path: &str) -> crate::Result<()> {
        self.inner.create_dir(&self.resolve(path)?)
    }

    fn metadata(&self, path: &str) -> crate::Result<Metadata> {
        self.inner.metadata(&self.resolve(path)?)
    }

    fn open_file_options(&self, path: &str, options: &OpenOptions) -> crate::Result<Box<dyn File>> {
        self.inner.open_file_options(&self.resolve(path)?, options)
    }

    fn read_dir(&self, path: &str) -> crate::Result<Box<dyn Iterator<Item = crate::Result<DirEntry>>>> {
        self.inner.read_dir(&self.resolve(path)?)
    }

    fn remove_dir(&self, path: &str) -> crate::Result<()> {
        self.inner.remove_dir(&self.resolve(path)?)
    }

    fn remove_file(&self, path: &str) -> crate::Result<()> {
        self.inner.remove_file(&self.resolve(path)?)
    }
}

#[cfg(test)]
mod test {
    use super::ScopedFS;
    use crate::memory_fs::MemoryFS;
    use crate::FileSystem;
    use std::io::{ErrorKind, Write};

    /// Builds a MemoryFS with:
    ///   src/init.luau   — "entry"
    ///   secret.txt      — "secret"  (outside scope)
    fn make_fs() -> ScopedFS<MemoryFS> {
        let mem = MemoryFS::default();
        mem.create_dir("src").unwrap();
        write!(mem.create_file("src/init.luau").unwrap(), "entry").unwrap();
        write!(mem.create_file("secret.txt").unwrap(), "secret").unwrap();
        ScopedFS::new("src", mem)
    }

    #[test]
    fn reads_file_inside_scope() {
        let fs = make_fs();
        let content = fs.open_file("init.luau").unwrap().read_into_string().unwrap();
        assert_eq!(content, "entry");
    }

    #[test]
    fn traversal_single_dotdot() {
        let fs = make_fs();
        let err = fs.open_file("../secret.txt").err().unwrap();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn traversal_dotdot_chain() {
        let fs = make_fs();
        let err = fs.open_file("../../secret.txt").err().unwrap();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn traversal_descend_then_escape() {
        // goes into a subdir first, then tries to climb out past root
        let fs = make_fs();
        let err = fs.open_file("subdir/../../secret.txt").err().unwrap();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn root_resolves_to_scope_root() {
        let fs = make_fs();
        // "/" or "" should resolve to "src" — a directory, not an error
        assert!(fs.metadata("/").unwrap().is_directory());
        assert!(fs.metadata("").unwrap().is_directory());
    }
}
