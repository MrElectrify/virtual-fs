use crate::file::{DirEntry, File, Metadata, OpenOptions};
use crate::memory_fs::MemoryFS;
use crate::util::{not_supported, parent_iter};
use crate::FileSystem;
use std::io::{Read, Write};
use std::path::Path;
use tar::{Archive, EntryType};

/// A filesystem mounted on a Tarball archive, backed by a Memory FS.
/// Because the FS is backed by memory, all files are immediately loaded
/// into memory, so `filtered` variants of constructors should be used
/// to avoid large files that may not need to be accessed.
pub struct TarFS {
    memory_fs: MemoryFS,
}

/// Filters over filesystems.
pub trait FileSystemFilter {
    /// Returns true if the path should be included in the filesystem.
    ///
    /// # Arguments
    /// `path`: THe path to the file.  
    fn should_include(&self, path: &Path) -> bool;
}

impl<F: Fn(&Path) -> bool> FileSystemFilter for F {
    fn should_include(&self, path: &Path) -> bool {
        self(path)
    }
}

impl TarFS {
    /// Creates a new tar-backed filesystem.
    ///
    /// # Arguments
    /// `archive`: The tarball archive itself.
    pub fn new<R: Read>(archive: R) -> crate::Result<Self> {
        Self::new_filtered(archive, |_: &_| true)
    }

    /// Creates a new tar-backed filesystem with filtered contents.
    ///
    /// # Arguments
    /// `archive`: The tarball archive itself.  
    /// `filter`: A filter that determines which entries are included in the filesystem.  
    pub fn new_filtered<R: Read, F: FileSystemFilter>(
        archive: R,
        filter: F,
    ) -> crate::Result<Self> {
        // iterate through each entry and build the memory FS
        let archive = Archive::new(archive);

        Self::build_fs(archive, filter).map(|fs| Self { memory_fs: fs })
    }

    /// Builds the memory file system from the archive.
    ///
    /// # Arguments
    /// `archive`: The archive itself.
    fn build_fs<R: Read, F: FileSystemFilter>(
        mut archive: Archive<R>,
        filter: F,
    ) -> crate::Result<MemoryFS> {
        let memory_fs = MemoryFS::default();

        // iterate over the archive and read in any files that don't already exist
        for entry in archive.entries()? {
            let mut entry = entry?;

            // ignore anything that isn't a regular folder
            if entry.header().entry_type() != EntryType::Regular {
                continue;
            }

            let entry_path = entry.path()?.into_owned();

            // ignore filtered files
            if !filter.should_include(&entry_path) {
                continue;
            }

            // recursively create parent directories
            for parent_path in parent_iter(&entry_path).map(Path::to_string_lossy).rev() {
                // only care about directories that exist
                if memory_fs.exists(&parent_path)? {
                    continue;
                }

                memory_fs.create_dir(&parent_path)?;
            }

            // read the entire entry to a vec
            let mut file_contents = Vec::with_capacity(entry.header().size()? as usize);
            entry.read_to_end(&mut file_contents)?;

            // create the file and write all of the contents
            let mut file = memory_fs.create_file(&format!("/{}", entry_path.to_string_lossy()))?;
            file.write_all(&file_contents)?;
        }

        Ok(memory_fs)
    }
}

impl FileSystem for TarFS {
    fn create_dir(&self, _path: &str) -> crate::Result<()> {
        Err(not_supported())
    }

    fn metadata(&self, path: &str) -> crate::Result<Metadata> {
        self.memory_fs.metadata(path)
    }

    fn open_file_options(&self, path: &str, options: &OpenOptions) -> crate::Result<Box<dyn File>> {
        if options.write {
            return Err(not_supported());
        }

        self.memory_fs.open_file_options(path, options)
    }

    fn read_dir(
        &self,
        path: &str,
    ) -> crate::Result<Box<dyn Iterator<Item = crate::Result<DirEntry>>>> {
        self.memory_fs.read_dir(path)
    }

    fn remove_dir(&self, _path: &str) -> crate::Result<()> {
        Err(not_supported())
    }

    fn remove_file(&self, _path: &str) -> crate::Result<()> {
        Err(not_supported())
    }
}

#[cfg(test)]
mod test {
    use std::fs::File;
    use std::io::Read;

    use crate::FileSystem;
    use xz::read::XzDecoder;

    use super::TarFS;

    #[test]
    fn bad_xz() {
        let file = File::open("test/bad.tar.xz").unwrap();
        let bad_archive = TarFS::new(XzDecoder::new(file));

        assert!(bad_archive.is_err());
    }

    #[test]
    fn single_file_xz_empty() {
        let file = File::open("test/empty.tar.xz").unwrap();
        let archive = TarFS::new(XzDecoder::new(file)).unwrap();

        let files = archive.read_dir("").unwrap().collect::<Vec<_>>();

        assert_eq!(files.len(), 1);

        let mut empty_file = archive.open_file("/empty").unwrap();
        let mut file_contents = vec![];
        empty_file.read_to_end(&mut file_contents).unwrap();

        assert_eq!(file_contents.len(), 0);
    }

    #[test]
    fn single_file_xz_not_empty() {
        let file = File::open("test/not_empty.tar.xz").unwrap();
        let archive = TarFS::new(XzDecoder::new(file)).unwrap();

        let files = archive.read_dir("").unwrap().collect::<Vec<_>>();

        assert_eq!(files.len(), 1);

        let mut file = archive.open_file("/not_empty").unwrap();
        let mut file_contents = String::new();
        file.read_to_string(&mut file_contents).unwrap();

        assert_eq!(file_contents, "something interesting\n");
    }

    #[test]
    fn deep_fs_xz() {
        let file = File::open("test/deep_fs.tar.xz").unwrap();
        let archive = TarFS::new(XzDecoder::new(file)).unwrap();

        let files = archive.read_dir("folder").unwrap().collect::<Vec<_>>();

        assert_eq!(files.len(), 2);

        let mut file = archive.open_file("/folder/and/it/desc").unwrap();
        let mut file_contents = String::new();
        file.read_to_string(&mut file_contents).unwrap();

        assert_eq!(file_contents, "it\n");
    }
}
