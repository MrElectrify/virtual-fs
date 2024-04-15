mod entry;
mod file;

use crate::file::{DirEntry, Metadata, OpenOptions};
use crate::memory_fs::file::{FileHandle, FileMode};
use crate::tree::{Directory, Entry, FilesystemTree};
use crate::util::{already_exists, invalid_path, not_found};
use crate::FileSystem;
use itertools::Itertools;
use parking_lot::Mutex;
use std::collections::{hash_map, HashMap};
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;

/// A file within the memory filesystem.
type File = Arc<Mutex<Vec<u8>>>;

/// A memory-backed filesystem. All files are stored within.
#[derive(Default)]
pub struct MemoryFS {
    inner: FilesystemTree<File>,
}

impl MemoryFS {
    fn with_parent_and_child_name<R, P: AsRef<Path>, F: FnOnce(&mut Directory<File>, &str) -> R>(
        &self,
        path: P,
        f: F,
    ) -> crate::Result<R> {
        let parent_directory = path.as_ref().parent().ok_or_else(invalid_path)?;
        let child_name = path
            .as_ref()
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(invalid_path)?;

        // fetch the parent directory and insert the new directory, if not already existent
        self.inner
            .with_directory(parent_directory, |dir| f(dir, child_name))
    }
}

impl FileSystem for MemoryFS {
    fn create_dir(&self, path: &str) -> crate::Result<()> {
        // fetch the parent directory and insert the new directory, if not already existent
        self.with_parent_and_child_name(path, |dir, directory_name| {
            match dir.entry(directory_name.to_owned()) {
                hash_map::Entry::Vacant(vac) => {
                    vac.insert(Entry::Directory(HashMap::default()));
                    Ok(())
                }
                _ => Err(already_exists()),
            }
        })?
    }

    fn metadata(&self, path: &str) -> crate::Result<Metadata> {
        // fetch the parent directory, because the entry can either be a folder or file
        self.with_parent_and_child_name(path, |dir, file_name| match dir.get(file_name) {
            Some(Entry::Directory(_)) => Ok(Metadata::directory()),
            Some(Entry::UserData(file)) => Ok(Metadata::file(file.lock().len() as u64)),
            None => Err(not_found()),
        })?
    }

    fn open_file_options(
        &self,
        path: &str,
        options: &OpenOptions,
    ) -> crate::Result<Box<dyn crate::File>> {
        // grab the file
        let mut file = self.with_parent_and_child_name(path, |dir, file_name| {
            let file = match dir.entry(file_name.to_owned()) {
                hash_map::Entry::Occupied(entry) => {
                    // of course we can only grab the file if it's a file
                    if let Entry::UserData(file) = entry.get() {
                        file.clone()
                    } else {
                        return Err(not_found());
                    }
                }
                hash_map::Entry::Vacant(vacant) => {
                    if options.create {
                        // create a new empty file and return it
                        let file = File::new(Mutex::default());
                        vacant.insert(Entry::UserData(file.clone()));
                        file
                    } else {
                        return Err(not_found());
                    }
                }
            };

            let mode = FileMode::from_options(options);
            Ok(FileHandle::new(file, mode))
        })??;

        // if we want to truncate the file, clear the contents
        if options.truncate {
            file.clear();
        }

        Ok(Box::new(file))
    }

    fn read_dir(
        &self,
        path: &str,
    ) -> crate::Result<Box<dyn Iterator<Item = crate::Result<DirEntry>>>> {
        self.inner.with_directory(path, |dir| {
            let iter: Box<dyn Iterator<Item = crate::Result<DirEntry>>> = Box::new(
                dir.iter()
                    .map(|(name, entry)| {
                        Ok(DirEntry {
                            path: name.into(),
                            metadata: entry.into(),
                        })
                    })
                    .collect_vec()
                    .into_iter(),
            );
            iter
        })
    }

    fn remove_dir(&self, path: &str) -> crate::Result<()> {
        self.with_parent_and_child_name(path, |parent, dir| match parent.entry(dir.to_owned()) {
            hash_map::Entry::Occupied(occ) if matches!(occ.get(), Entry::Directory(_)) => {
                occ.remove();
                Ok(())
            }
            _ => Err(not_found()),
        })?
    }

    fn remove_file(&self, path: &str) -> crate::Result<()> {
        self.with_parent_and_child_name(path, |parent, dir| match parent.entry(dir.to_owned()) {
            hash_map::Entry::Occupied(occ) if matches!(occ.get(), Entry::UserData(_)) => {
                occ.remove();
                Ok(())
            }
            _ => Err(not_found()),
        })?
    }

    fn create_dir_all(&self, path: &str) -> crate::Result<()> {
        self.inner.create_dir_all(path, |_| ())
    }
}

#[cfg(test)]
mod test {
    use crate::file::{FileType, Metadata};
    use crate::memory_fs::MemoryFS;
    use crate::FileSystem;
    use std::collections::BTreeMap;
    use std::io::Write;

    fn memory_fs() -> MemoryFS {
        let fs = MemoryFS::default();

        write!(fs.create_file("file").unwrap(), "something interesting").unwrap();
        fs.create_dir_all("folder/and/it/goes/deeper").unwrap();
        write!(fs.create_file("folder/and/it/goes/desc").unwrap(), "goes").unwrap();

        fs
    }

    fn read_directory(fs: &MemoryFS, dir: &str) -> BTreeMap<String, Metadata> {
        fs.read_dir(dir)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.path.to_str().unwrap().to_owned(), entry.metadata)
            })
            .collect()
    }

    #[test]
    fn create_file() {
        memory_fs();
    }

    #[test]
    fn metadata() {
        let fs = memory_fs();

        // basic file
        for name in ["file", "/file", "./file", "test/../file"] {
            let md = fs.metadata(name).unwrap();
            assert_eq!(md.file_type, FileType::File);
            assert_eq!(md.len, 21);
        }

        // basic folder
        for name in ["folder", "/folder", "./folder", "test/../folder"] {
            let md = fs.metadata(name).unwrap();
            assert_eq!(md.file_type, FileType::Directory);
            assert_eq!(md.len, 0);
        }

        // nested file
        for name in [
            "folder/and/it/goes/desc",
            "/folder/and/it/goes/desc",
            "./folder/and/it/goes/desc",
            "test/../folder/and/it/goes/desc",
        ] {
            let md = fs.metadata(name).unwrap();
            assert_eq!(md.file_type, FileType::File);
            assert_eq!(md.len, 4);
        }
    }

    #[test]
    fn read_dir() {
        let fs = memory_fs();

        // simple
        for name in ["", "/", "./", "//", "\\"] {
            let files = read_directory(&fs, name);
            itertools::assert_equal(files.keys(), vec!["file", "folder"]);
            itertools::assert_equal(
                files.values(),
                vec![&Metadata::file(21), &Metadata::directory()],
            )
        }

        // nested
        for name in [
            "folder/and/it/goes",
            "/folder/and/it/goes",
            "./folder/and/it/goes/",
            "///folder/and/it/goes///",
            "\\folder\\and\\it\\goes\\",
        ] {
            let files = read_directory(&fs, name);
            itertools::assert_equal(files.keys(), vec!["deeper", "desc"]);
            itertools::assert_equal(
                files.values(),
                vec![&Metadata::directory(), &Metadata::file(4)],
            )
        }

        // traversal
        for name in [
            "folder/and/../..",
            "./folder/and/../..",
            ".//folder/and//../..",
            "\\folder//../folder/and/../..",
        ] {
            let files = read_directory(&fs, name);
            itertools::assert_equal(files.keys(), vec!["file", "folder"]);
            itertools::assert_equal(
                files.values(),
                vec![&Metadata::file(21), &Metadata::directory()],
            )
        }
    }

    #[test]
    fn remove_dir() {
        let fs = memory_fs();

        assert!(fs.exists("folder/and/it/goes").unwrap());
        fs.remove_dir("folder/and/it").unwrap();
        assert!(!fs.exists("folder/and/it/goes").unwrap());
        assert!(!fs.exists("/folder/and/it").unwrap());
        assert!(!fs.exists("/folder/and/it/goes/desc").unwrap());
    }

    #[test]
    fn remove_file() {
        let fs = memory_fs();

        assert!(fs.exists("folder/and/it/goes/desc").unwrap());
        fs.remove_file("folder/and/it/goes/desc").unwrap();
        assert!(fs.exists("folder/and/it/goes/deeper").unwrap());
        assert!(!fs.exists("folder/and/it/goes/desc").unwrap());
    }
}
