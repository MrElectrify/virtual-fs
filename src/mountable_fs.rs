use crate::file::{DirEntry, File, Metadata, OpenOptions};
use crate::tree::{normalize_and_relativize, Entry, FilesystemTree};
use crate::util::{already_exists, invalid_path, not_found, not_supported};
use crate::FileSystem;
use itertools::Itertools;
use std::collections::hash_map;
use std::ffi::OsStr;
use std::path::Path;

type FS = Box<dyn FileSystem + Send + Sync>;

/// A filesystem that supports the mounting of other filesystems at designated paths (excluding the root).
#[derive(Default)]
pub struct MountableFS {
    inner: FilesystemTree<FS>,
}

impl MountableFS {
    /// Mounts a filesystem at the given path.
    ///
    /// # Arguments
    /// `path`: The path to mount the filesystem at.  
    /// `fs`: The filesystem to mount.  
    pub fn mount<P: AsRef<Path>>(&self, path: P, fs: Box<dyn FileSystem + Send + Sync>) -> crate::Result<()> {
        // find the parent path
        let normalized_path = normalize_and_relativize(path);
        let parent_path = normalized_path.parent().ok_or_else(invalid_path)?;
        let child_path = normalized_path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(invalid_path)?;

        // create the parent path
        self.inner.create_dir_all(parent_path, |dir| {
            if let hash_map::Entry::Vacant(vac) = dir.entry(child_path.to_owned()) {
                vac.insert(Entry::UserData(fs));
                Ok(())
            } else {
                Err(already_exists())
            }
        })??;

        Ok(())
    }
}

impl<'a> FromIterator<(&'a str, Box<dyn FileSystem + Send + Sync>)> for MountableFS {
    fn from_iter<T: IntoIterator<Item = (&'a str, Box<dyn FileSystem + Send + Sync>)>>(iter: T) -> Self {
        let mountable_fs = Self::default();
        for (path, fs) in iter {
            mountable_fs.mount(path, fs).unwrap();
        }
        mountable_fs
    }
}

impl FileSystem for MountableFS {
    fn create_dir(&self, _path: &str) -> crate::Result<()> {
        Err(not_supported())
    }

    fn metadata(&self, path: &str) -> crate::Result<Metadata> {
        self.inner.with_entry(path, |maybe_directory| {
            match maybe_directory {
                Ok(_dir) => Ok(Metadata::directory()),
                Err((fs, remaining_path)) => {
                    if remaining_path.as_os_str().is_empty() {
                        // the root directory of a filesystem is a directory
                        Ok(Metadata::directory())
                    } else {
                        // `remaining_path` is derived from `path`, so this is safe
                        fs.metadata(remaining_path.to_str().unwrap())
                    }
                }
            }
        })
    }

    fn open_file_options(&self, path: &str, options: &OpenOptions) -> crate::Result<Box<dyn File>> {
        self.inner.with_entry(path, |maybe_directory| {
            maybe_directory
                .err()
                .map(|(fs, remaining_path)| {
                    // `remaining_path` is derived from `path`, so this is safe
                    fs.open_file_options(remaining_path.to_str().unwrap(), options)
                })
                .ok_or_else(not_found)
        })?
    }

    fn read_dir(
        &self,
        path: &str,
    ) -> crate::Result<Box<dyn Iterator<Item = crate::Result<DirEntry>>>> {
        self.inner
            .with_entry(path, |maybe_entry| match maybe_entry {
                Ok(dir) => {
                    // we should have a directory
                    let entries = dir
                        .iter()
                        .map(|(path, _)| {
                            // filesystems and directories are both functionally directories
                            Ok(DirEntry {
                                path: path.into(),
                                metadata: Metadata::directory(),
                            })
                        })
                        .collect_vec();

                    Ok::<Box<dyn Iterator<Item = crate::Result<DirEntry>>>, _>(Box::new(
                        entries.into_iter(),
                    ))
                }
                Err((fs, remaining_path)) => {
                    // `remaining_path` is derived from `path`, so this is safe
                    fs.read_dir(remaining_path.to_str().unwrap())
                }
            })
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
    use crate::file::Metadata;
    use crate::memory_fs::MemoryFS;
    use crate::mountable_fs::MountableFS;
    use crate::util::test::read_directory;
    use crate::{FileSystem, MockFileSystem};
    use std::io::Write;

    const TEST_PATHS: [&str; 4] = [
        "test/abc",
        "/test/abc",
        "./test//abc",
        "//test\\def//../abc",
    ];

    #[test]
    fn mount() {
        for mount_point in TEST_PATHS {
            let fs = MountableFS::default();
            assert!(!fs.exists("test/abc").unwrap());

            fs.mount(mount_point, Box::new(MockFileSystem::new()))
                .unwrap();
            assert!(fs.exists("test/abc").unwrap());
        }
    }

    #[test]
    fn double_mount() {
        for mount_point in TEST_PATHS {
            let fs = MountableFS::default();
            fs.mount(mount_point, Box::new(MockFileSystem::new()))
                .unwrap();
            assert!(fs
                .mount(mount_point, Box::new(MockFileSystem::new()))
                .is_err())
        }
    }

    fn mounted_fs() -> MountableFS {
        let fs = MountableFS::default();

        let memory_fs = MemoryFS::default();
        write!(memory_fs.create_file("abc").unwrap(), "file").unwrap();
        memory_fs.create_dir_all("folder/and/it").unwrap();
        fs.mount("test", Box::new(memory_fs)).unwrap();

        fs
    }

    #[test]
    fn metadata() {
        let fs = mounted_fs();

        for path in TEST_PATHS {
            assert_eq!(fs.metadata(path).unwrap(), Metadata::file(4));
        }

        assert_eq!(fs.metadata("test/folder").unwrap(), Metadata::directory());
    }

    #[test]
    fn open_file() {
        let fs = mounted_fs();

        for path in TEST_PATHS {
            assert_eq!(
                fs.open_file(path).unwrap().read_into_string().unwrap(),
                "file"
            );
        }

        assert!(fs.open_file("folder").is_err());
    }

    #[test]
    fn read_dir() {
        let fs = mounted_fs();

        for path in ["/", "//", "", ".", "./", "test/something/else/../../../"] {
            let dir = read_directory(&fs, path);
            itertools::assert_equal(dir.keys(), vec!["test"]);
            itertools::assert_equal(dir.values(), vec![&Metadata::directory()])
        }

        for path in ["/test", "./test/", "\\test/\\", "test/../test//"] {
            let dir = read_directory(&fs, path);
            itertools::assert_equal(dir.keys(), vec!["abc", "folder"]);
            itertools::assert_equal(
                dir.values(),
                vec![&Metadata::file(4), &Metadata::directory()],
            )
        }
    }

    #[test]
    fn exists() {
        let fs = mounted_fs();

        for path in ["/", "//", "", ".", "./", "test/something/else/../../../"] {
            assert!(fs.exists(path).unwrap());
        }

        for path in TEST_PATHS {
            assert!(fs.exists(path).unwrap());
        }

        assert!(!fs.exists("nonsense").unwrap());
        assert!(!fs.exists("test/nonsense").unwrap());
        assert!(fs.exists("test/folder").unwrap());
        assert!(fs.exists("test/folder/and/").unwrap());
    }
}
