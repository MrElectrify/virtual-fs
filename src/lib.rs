//! # Virtual Filesystems for Rust
//! This crate defines and implements various virtual filesystems for Rust. It's loosely inspired by the `vfs` crate with
//! a focus on conformity with `std`.
//!
//! `virtual-fs` has the following FileSystems implemented out of the box:
//! - `PhysicalFS`: A read-write physical filesystem mounted at a directory. Path traversal outside the root is permitted.
//! - `SandboxedPhysicalFS`: A read-write physical filesystem that guards against traversal through backtracking and symbolic link
//! traversal.
//! - `MemoryFS`: A read-write in-memory filesystem.
//! - `RocFS`: A "read-only collection" filesystem. This filesystem is similar to `OverlayFS`, but is read-only. This
//! filesystem searches filesystems in mount-order for files, allowing multiple filesystems to be mounted at once.
//! - `MountableFS`: A read-write filesystem that supports mounting other filesystems at given paths.
//! - `ZipFS`: A read-only filesystem that mounts a ZIP archive, backed by the `zip` crate.
//! - `TarFS` A read-only filesystem that mounts a Tarball, backed by the `tar` crate.

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
