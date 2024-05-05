use crate::file::{DirEntry, File, FileType, Metadata, OpenOptions};
use crate::util::{make_relative, not_found, not_supported, parent_iter};
use crate::{util, FileSystem};
use itertools::Itertools;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::io;
use std::io::{Cursor, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zip::read::ZipFile;
use zip::result::{ZipError, ZipResult};
use zip::ZipArchive;

/// A virtual FileSystem backed by a ZIP file. Only supports read operations for now.
#[derive(Debug)]
pub struct ZipFS<R: Read + Seek> {
    zip_file: Mutex<ZipArchive<R>>,
    directories: HashSet<PathBuf>,
}

impl<R: Read + Seek> ZipFS<R> {
    /// Mounts a ZIP file onto the local filesystem.
    pub fn new(zip_file: R) -> ZipResult<Self> {
        let zip_file = ZipArchive::new(zip_file)?;

        // collect folders
        let mut directories = HashSet::from_iter([Path::new("").to_owned()]);
        for file_name in zip_file.file_names() {
            for parent in parent_iter(Path::new(file_name)) {
                directories.insert(parent.to_owned());
            }
        }

        Ok(Self {
            zip_file: Mutex::new(zip_file),
            directories,
        })
    }

    fn convert_error<T>(maybe_error: ZipResult<T>) -> crate::Result<T> {
        maybe_error.map_err(|err| match err {
            ZipError::FileNotFound => {
                io::Error::new(ErrorKind::NotFound, "File not found in zip archive")
            }
            ZipError::Io(io_error) => io_error,
            ZipError::InvalidArchive(error_str) => {
                io::Error::new(ErrorKind::InvalidData, error_str)
            }
            ZipError::UnsupportedArchive(error_str) => {
                io::Error::new(ErrorKind::Unsupported, error_str)
            }
        })
    }

    fn normalize_path<P: AsRef<Path>>(path: P) -> PathBuf {
        // as far as I can tell, zip files are relative from the root
        make_relative(util::normalize_path(path))
    }

    #[cfg(not(feature = "fallback_search"))]
    fn with_file<RV, F: FnOnce(ZipFile) -> RV>(
        &self,
        normalized_path: &Path,
        f: F,
    ) -> crate::Result<RV> {
        let mut zip_file = self.zip_file.lock();

        let entry = Self::convert_error(
            zip_file.by_name(normalized_path.to_str().ok_or_else(not_supported)?),
        )?;
        Ok(f(entry))
    }

    #[cfg(feature = "fallback_search")]
    fn with_file<RV, F: FnOnce(ZipFile) -> RV>(
        &self,
        normalized_path: &Path,
        f: F,
    ) -> crate::Result<RV> {
        let mut zip_file = self.zip_file.lock();
        // this is a bit strange because of lifetimes. even in `Err(FileNotFound)` the borrow checker still considers
        // `zip_file` as borrowed mutably, so we have to leave that block entirely.
        match zip_file.by_name(&normalized_path.to_string_lossy()) {
            Ok(entry) => return Ok(f(entry)),
            Err(ZipError::FileNotFound) => {}
            Err(err) => return Self::convert_error(Err(err)),
        }

        // as a fallback, search for it in O(n) time, case-insensitive
        let file_name = zip_file
            .file_names()
            .find(|name| {
                normalized_path
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&Self::normalize_path(name).to_string_lossy())
            })
            .ok_or_else(not_found)?
            .to_owned();
        let entry = Self::convert_error(zip_file.by_name(&file_name))?;
        Ok(f(entry))
    }
}

impl<R: Read + Seek> FileSystem for ZipFS<R> {
    fn create_dir(&self, _path: &str) -> crate::Result<()> {
        Err(not_supported())
    }

    fn metadata(&self, path: &str) -> crate::Result<Metadata> {
        let normalized_path = Self::normalize_path(path);

        // try directories first
        if self.directories.get(normalized_path.as_path()).is_some() {
            return Ok(Metadata {
                file_type: FileType::Directory,
                len: 0,
            });
        }

        // now files
        self.with_file(normalized_path.as_path(), |file| Metadata {
            file_type: FileType::File,
            len: file.size(),
        })
    }

    fn open_file_options(&self, path: &str, options: &OpenOptions) -> crate::Result<Box<dyn File>> {
        // ensure we only want to read
        if !options.read || options.write {
            return Err(not_supported());
        }

        // open the file and read into a readable buffer
        self.with_file::<crate::Result<Box<dyn File>>, _>(
            &Self::normalize_path(path),
            |mut entry| {
                let mut contents = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut contents)?;
                Ok(Box::new(ZipFileContents {
                    inner: Cursor::new(contents),
                }))
            },
        )?
    }

    fn read_dir(
        &self,
        path: &str,
    ) -> crate::Result<Box<dyn Iterator<Item = crate::Result<DirEntry>>>> {
        let directory = Self::normalize_path(path);

        // if there are no folders with this path, error out
        if !self.directories.contains(&directory) {
            return Err(not_found());
        }

        let mut zip_file = self.zip_file.lock();
        let mut files = HashMap::new();
        for file in zip_file
            .file_names()
            .map(|file_name| file_name.to_owned())
            .collect_vec()
        {
            let normalized_file = Self::normalize_path(&file);

            let mut add_parent = |normalized_path: &Path, metadata| {
                if normalized_path.parent()? == directory {
                    files.insert(PathBuf::from(normalized_path.file_name()?), metadata);
                }

                Some(())
            };

            // if the file's parent is the directory, it's in the directory
            add_parent(
                &normalized_file,
                Metadata::file(zip_file.by_name(&file)?.size()),
            );

            // if the file's parent directory is in the directory, add it
            if let Some(file_parent) = normalized_file.parent() {
                add_parent(file_parent, Metadata::directory());
            }
        }

        Ok(Box::new(
            files
                .into_iter()
                .map(|(path, metadata)| Ok(DirEntry { path, metadata })),
        ))
    }

    fn remove_dir(&self, _path: &str) -> crate::Result<()> {
        Err(not_supported())
    }

    fn remove_file(&self, _path: &str) -> crate::Result<()> {
        Err(not_supported())
    }
}

struct ZipFileContents {
    inner: Cursor<Vec<u8>>,
}

impl Read for ZipFileContents {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for ZipFileContents {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl Write for ZipFileContents {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(not_supported())
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(not_supported())
    }
}

impl File for ZipFileContents {
    fn metadata(&self) -> crate::Result<Metadata> {
        Ok(Metadata::file(self.inner.get_ref().len() as u64))
    }
}

#[cfg(test)]
mod test {
    use crate::file::{FileType, Metadata};
    use crate::zip_fs::ZipFS;
    use crate::FileSystem;
    use std::collections::BTreeMap;
    use std::fs::File;

    fn read_directory(fs: &ZipFS<File>, path: &str) -> crate::Result<BTreeMap<String, Metadata>> {
        Ok(fs
            .read_dir(path)?
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.path.to_str().unwrap().to_owned(), entry.metadata)
            })
            .collect::<BTreeMap<_, _>>())
    }

    fn zip_fs() -> ZipFS<File> {
        ZipFS::new(File::open("test/deep_fs.zip").unwrap()).unwrap()
    }

    #[test]
    fn read_dir() {
        let fs = zip_fs();

        let root = read_directory(&fs, "").unwrap();
        itertools::assert_equal(root.keys(), vec!["file", "folder"]);
        itertools::assert_equal(
            root.values().map(|md| md.file_type),
            vec![FileType::File, FileType::Directory],
        );
        itertools::assert_equal(root.values().map(|md| md.len), vec![2571, 0]);

        let another_root = read_directory(&fs, ".").unwrap();
        assert_eq!(root, another_root);

        let another_root = read_directory(&fs, "///").unwrap();
        assert_eq!(root, another_root);

        let another_root = read_directory(&fs, "\\").unwrap();
        assert_eq!(root, another_root);

        let another_root = read_directory(&fs, "///test/../").unwrap();
        assert_eq!(root, another_root);

        let deeper_root = read_directory(&fs, "folder/and/it").unwrap();
        itertools::assert_equal(deeper_root.keys(), vec!["desc", "goes"]);

        assert!(read_directory(&fs, "file").is_err());
        assert!(read_directory(&fs, "not_a_real_path").is_err());
    }

    #[test]
    fn open_file() {
        let fs = zip_fs();

        let mut file = fs.open_file("file").unwrap();
        let md = file.metadata().unwrap();
        assert_eq!(md.file_type, FileType::File);
        assert_eq!(md.len, 2571);

        let file = file.read_into_string().unwrap();
        assert!(file.starts_with("Lorem ipsum dolor"));

        let indirect_file = fs
            .open_file("///something/..\\file")
            .unwrap()
            .read_into_string()
            .unwrap();
        assert_eq!(indirect_file, file);

        let nested_file = fs
            .open_file("folder/and/it/goes/deeper/desc")
            .unwrap()
            .read_into_string()
            .unwrap();
        assert_eq!(nested_file, "deeper\n")
    }

    #[test]
    fn metadata() {
        let fs = zip_fs();

        let md = fs.metadata("file").unwrap();
        assert_eq!(md.file_type, FileType::File);
        assert_eq!(md.len, 2571);

        let md = fs.metadata("folder").unwrap();
        assert_eq!(md.file_type, FileType::Directory);
        assert_eq!(md.len, 0);

        let md = fs.metadata("folder/and/it/goes/desc").unwrap();
        assert_eq!(md.file_type, FileType::File);
        assert_eq!(md.len, 5);
    }

    #[test]
    fn exists() {
        let fs = zip_fs();

        assert!(fs.exists("/").unwrap());
        assert!(fs.exists("").unwrap());
        assert!(fs.exists("file").unwrap());
        #[cfg(feature = "fallback_search")]
        assert!(fs.exists("FiLe").unwrap());
        assert!(!fs.exists("no_file").unwrap());
        assert!(fs.exists("folder").unwrap());
        #[cfg(feature = "fallback_search")]
        assert!(fs.exists("folDeR").unwrap());
        assert!(fs.exists("folder/and/it").unwrap());
        #[cfg(feature = "fallback_search")]
        assert!(fs.exists("folder/anD/iT").unwrap());
        assert!(fs.exists("folder/and/it/desc").unwrap());
        assert!(!fs.exists("folder/and/it/does/not").unwrap());
        assert!(fs.exists("///test/something_else/../../file").unwrap());
        #[cfg(feature = "fallback_search")]
        assert!(fs.exists("///test/something_elsE/../../file").unwrap());
    }
}
