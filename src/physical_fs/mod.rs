mod path_resolver;

use crate::file::{DirEntry, File, Metadata, OpenOptions};
use crate::physical_fs::path_resolver::{
    PathResolver, SandboxedPathResolver, UnrestrictedPathResolver,
};
use crate::util::invalid_path;
use crate::FileSystem;
use normalize_path::NormalizePath;
use std::fs;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

/// The physical filesystem, backed by a root on the drive.
pub struct PhysicalFSImpl<R: PathResolver> {
    root: PathBuf,
    _marker: PhantomData<R>,
}

/// The physical filesystem, backed by a root on the drive. This filesystem will not protect against
/// directory traversal and very simply appends the target path to the root.
pub type PhysicalFS = PhysicalFSImpl<UnrestrictedPathResolver>;
/// The physical filesystem, backed by a root on the drive. This filesystem will perform basic
/// protections against directory traversal in the form of returning an error if a user tries to
/// escape the current directory.
pub type SandboxedPhysicalFS = PhysicalFSImpl<SandboxedPathResolver>;

impl<R: PathResolver> PhysicalFSImpl<R> {
    /// Creates a new physical file system at the given root.
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        Self {
            root: root.as_ref().normalize(),
            _marker: PhantomData,
        }
    }
}

impl<R: PathResolver> FileSystem for PhysicalFSImpl<R> {
    fn create_dir(&self, path: &str) -> crate::Result<()> {
        fs::create_dir(R::resolve_path(&self.root, path)?)
    }

    fn metadata(&self, path: &str) -> crate::Result<Metadata> {
        fs::metadata(R::resolve_path(&self.root, path)?).map(Metadata::from)
    }

    fn open_file_options(&self, path: &str, options: &OpenOptions) -> crate::Result<Box<dyn File>> {
        fs::OpenOptions::from(options)
            .open(R::resolve_path(&self.root, path)?)
            .map::<Box<dyn File>, _>(|file| Box::new(file))
    }

    fn read_dir(
        &self,
        path: &str,
    ) -> crate::Result<Box<dyn Iterator<Item = crate::Result<DirEntry>>>> {
        Ok(Box::new(
            fs::read_dir(R::resolve_path(&self.root, path)?)?.map({
                let root = self.root.clone();
                move |entry| {
                    entry.and_then({
                        |entry| {
                            Ok(DirEntry {
                                // strip the root
                                path: entry
                                    .path()
                                    .strip_prefix(&root)
                                    .map_err(|_| invalid_path())?
                                    .into(),
                                metadata: entry.metadata()?.into(),
                            })
                        }
                    })
                }
            }),
        ))
    }

    fn remove_dir(&self, path: &str) -> crate::Result<()> {
        fs::remove_dir(R::resolve_path(&self.root, path)?)
    }

    fn remove_file(&self, path: &str) -> crate::Result<()> {
        fs::remove_file(R::resolve_path(&self.root, path)?)
    }
}

impl File for fs::File {
    fn metadata(&self) -> crate::Result<Metadata> {
        self.metadata().map(Metadata::from)
    }
}

#[cfg(test)]
mod test {
    use crate::file::FileType;
    use crate::physical_fs::{PhysicalFS, SandboxedPhysicalFS};
    use crate::FileSystem;
    use std::path::Path;

    fn physical_fs<P: AsRef<Path>>(root: P) -> (PhysicalFS, SandboxedPhysicalFS) {
        (
            PhysicalFS::new(root.as_ref()),
            SandboxedPhysicalFS::new(root.as_ref()),
        )
    }

    #[test]
    fn read_dir() {
        let (unrestricted_fs, sandboxed_fs) = physical_fs("test");

        // basic traversal
        let dir = sandboxed_fs.read_dir(".").unwrap();
        assert!(dir.count() > 0);
        let dir = unrestricted_fs.read_dir(".").unwrap();
        assert!(dir.count() > 0);

        // project root traversal
        assert!(sandboxed_fs.read_dir("..").is_err());
        let dir = unrestricted_fs.read_dir("..").unwrap();
        assert!(dir.count() > 0);

        // fancy project root traversal
        assert!(sandboxed_fs.read_dir("test/something/../../..").is_err());
        let dir = unrestricted_fs.read_dir("test/something/../../..").unwrap();
        assert!(dir.count() > 0);
    }

    #[test]
    fn metadata() {
        let (unrestricted_fs, sandboxed_fs) = physical_fs("test/folder_a");

        // basic traversal
        let md = sandboxed_fs.metadata(".").unwrap();
        assert_eq!(md.file_type, FileType::Directory);
        assert_eq!(md.len, 0);
        let md = unrestricted_fs.metadata(".").unwrap();
        assert_eq!(md.file_type, FileType::Directory);
        assert_eq!(md.len, 0);
        let md = sandboxed_fs.metadata("file_a").unwrap();
        assert_eq!(md.file_type, FileType::File);
        assert_eq!(md.len, 6);
        let md = unrestricted_fs.metadata("file_a").unwrap();
        assert_eq!(md.file_type, FileType::File);
        assert_eq!(md.len, 6);

        // project root traversal
        assert!(sandboxed_fs.metadata("../deep_fs.zip").is_err());
        let md = unrestricted_fs.metadata("../deep_fs.zip").unwrap();
        assert_eq!(md.file_type, FileType::File);
        assert_eq!(md.len, 2691);

        // fancy project root traversal
        assert!(sandboxed_fs.metadata("test/../../deep_fs.zip").is_err());
        let md = unrestricted_fs.metadata("test/../../deep_fs.zip").unwrap();
        assert_eq!(md.file_type, FileType::File);
        assert_eq!(md.len, 2691);
    }

    #[test]
    fn open_file() {
        let (unrestricted_fs, sandboxed_fs) = physical_fs("test/folder_a");

        // basic traversal
        let mut file = sandboxed_fs.open_file("file_a").unwrap();
        let md = file.metadata().unwrap();
        assert_eq!(md.file_type, FileType::File);
        assert_eq!(md.len, 6);
        assert_eq!(file.read_into_string().unwrap(), "file a");
        let mut file = unrestricted_fs.open_file("file_a").unwrap();
        let md = file.metadata().unwrap();
        assert_eq!(md.file_type, FileType::File);
        assert_eq!(md.len, 6);
        assert_eq!(file.read_into_string().unwrap(), "file a");

        // project root traversal
        assert!(sandboxed_fs.read_dir("../bad.tar.xz").is_err());
        let mut file = unrestricted_fs.open_file("../bad.tar.xz").unwrap();
        let md = file.metadata().unwrap();
        assert_eq!(md.file_type, FileType::File);
        assert_eq!(md.len, 4);
        assert_eq!(file.read_into_string().unwrap(), "abcd");

        // fancy project root traversal
        assert!(sandboxed_fs.read_dir("test/../../bad.tar.xz").is_err());
        let mut file = unrestricted_fs.open_file("test/../../bad.tar.xz").unwrap();
        let md = file.metadata().unwrap();
        assert_eq!(md.file_type, FileType::File);
        assert_eq!(md.len, 4);
        assert_eq!(file.read_into_string().unwrap(), "abcd");
    }

    #[test]
    fn exists() {
        let (unrestricted_fs, sandboxed_fs) = physical_fs("test");

        assert!(unrestricted_fs.exists("").unwrap());
        assert!(sandboxed_fs.exists("").unwrap());
        assert!(unrestricted_fs.exists(".").unwrap());
        assert!(sandboxed_fs.exists(".").unwrap());
        assert!(unrestricted_fs.exists("///").unwrap());
        assert!(sandboxed_fs.exists("///").unwrap());
        assert!(unrestricted_fs.exists("\\\\").unwrap());
        assert!(sandboxed_fs.exists("\\\\").unwrap());
        assert!(unrestricted_fs.exists("folder_a").unwrap());
        assert!(sandboxed_fs.exists("folder_a").unwrap());
        assert!(!unrestricted_fs.exists("folder_c").unwrap());
        assert!(!sandboxed_fs.exists("folder_c").unwrap());
        assert!(unrestricted_fs.exists("bad.tar.xz").unwrap());
        assert!(sandboxed_fs.exists("bad.tar.xz").unwrap());
        assert!(unrestricted_fs.exists("../Cargo.toml").unwrap());
        assert!(sandboxed_fs.exists("../Cargo.toml").is_err());
        assert!(!unrestricted_fs.exists("../Cargo.toml2").unwrap());
        assert!(sandboxed_fs.exists("../Cargo.toml2").is_err());
        assert!(unrestricted_fs.exists("folder_a/../../Cargo.toml").unwrap());
        assert!(sandboxed_fs.exists("folder_a/../../Cargo.toml").is_err());
    }
}
