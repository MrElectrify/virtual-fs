//! `vfs-extensions` provides some additional Virtual FileSystems to handle the following use cases:
//! - Tarball-backed filesystems - see `TarFS`.
//! - Zip-backed filesystems - see `ZipFS`.
//! - Read-only collection filesystems - see `RocFS`.

use crate::file::{DirEntry, File, Metadata, OpenOptions};
use mockall::automock;
use std::io::ErrorKind;

pub use error::*;

/// A file system with a directory tree.
#[automock]
pub trait FileSystem {
    /// Creates a directory at `path`.
    fn create_dir(&self, path: &str) -> Result<()>;
    /// Returns the metadata for the file/folder at `path.
    fn metadata(&self, path: &str) -> Result<Metadata>;
    /// Opens a file at `path` with options `options`.
    fn open_file_options(&self, path: &str, options: &OpenOptions) -> Result<Box<dyn File>>;
    /// Lists the files and folders contained in the directory denoted by `path`.
    fn read_dir(&self, path: &str) -> Result<Box<dyn Iterator<Item = Result<DirEntry>>>>;
    /// Removes the directory at `path`.
    fn remove_dir(&self, path: &str) -> Result<()>;
    /// Removes a file at `path`.
    fn remove_file(&self, path: &str) -> Result<()>;

    /// Creates a directory `path` and all of its parents.
    fn create_dir_all(&self, path: &str) -> Result<()> {
        util::create_dir_all(self, path)
    }
    /// Creates a file at `path` in write mode. The file will be opened in truncate mode, so all contents will be
    /// overwritten. If this is not desirable, use `open_file` directly.
    fn create_file(&self, path: &str) -> Result<Box<dyn File>> {
        self.open_file_options(path, &OpenOptions::default().create(true).truncate(true))
    }
    /// Returns `Ok(true)` or `Ok(false)` if a file or folder at `path` does or does not exist, and `Err(_)` if the
    /// presence cannot be verified.  
    fn exists(&self, path: &str) -> Result<bool> {
        match self.metadata(path) {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        }
    }
    /// Opens a file at `path` for reading.
    fn open_file(&self, path: &str) -> Result<Box<dyn File>> {
        self.open_file_options(path, &OpenOptions::default())
    }
}

pub mod error;
pub mod file;
pub mod memory_fs;
pub mod mountable_fs;
pub mod physical_fs;
pub mod roc_fs;
pub mod tar_fs;
mod tree;
pub mod util;
pub mod zip_fs;
