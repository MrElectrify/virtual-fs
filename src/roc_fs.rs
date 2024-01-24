use crate::file::{DirEntry, File, Metadata, OpenOptions};
use crate::util::{not_found, not_supported};
use crate::FileSystem;
use itertools::Itertools;
use std::io::ErrorKind;

/// "Read-only collection" filesystem. Does not support writing, but supports reading from any
/// of the layers. Differs from `OverlayFS` in that it only supports reading and is much less
/// complex and doesn't need to write a `.whiteout` directory that can sometimes prove problematic.
pub struct RocFS {
    pub layers: Vec<Box<dyn FileSystem>>,
}

impl RocFS {
    /// Creates a new read-only collection filesystem from layers. Layers will be traversed in order
    /// of their appearance in the vector.
    ///
    /// # Argument
    /// `layers`: The layers of the filesystem.
    pub fn new(layers: Vec<Box<dyn FileSystem>>) -> Self {
        Self { layers }
    }

    /// Checks each layer for a successful result.
    ///
    /// # Arguments
    /// `f`: The filesystem method.  
    /// `path`: The path invoked.  
    fn for_each_layer<R, F: Fn(&dyn FileSystem, &str) -> crate::Result<R>>(
        &self,
        f: F,
        path: &str,
    ) -> crate::Result<R> {
        for layer in &self.layers {
            match f(&**layer, path) {
                Ok(path) => return Ok(path),
                Err(err) if err.kind() == ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            }
        }

        Err(not_found())
    }
}

impl FileSystem for RocFS {
    fn create_dir(&self, _path: &str) -> crate::Result<()> {
        Err(not_supported())
    }

    fn metadata(&self, path: &str) -> crate::Result<Metadata> {
        self.for_each_layer(|layer, path| layer.metadata(path), path)
    }

    fn open_file_options(&self, path: &str, options: &OpenOptions) -> crate::Result<Box<dyn File>> {
        self.for_each_layer(|layer, path| layer.open_file_options(path, options), path)
    }

    fn read_dir(
        &self,
        path: &str,
    ) -> crate::Result<Box<dyn Iterator<Item = crate::Result<DirEntry>>>> {
        Ok(Box::new(
            self.layers
                .iter()
                .map(|layer| layer.read_dir(path))
                .filter(|res| {
                    res.as_ref()
                        .err()
                        .map(|err| err.kind() != ErrorKind::NotFound)
                        .unwrap_or(true)
                })
                .flatten_ok()
                .try_collect::<_, Vec<_>, _>()?
                .into_iter(),
        ))
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
    use crate::physical_fs::PhysicalFS;
    use crate::roc_fs::RocFS;
    use crate::util::test::read_directory;
    use crate::FileSystem;
    use std::io::ErrorKind;

    #[test]
    fn read_dir_happy_case() {
        let folder_a = PhysicalFS::new("test/folder_a");
        let folder_b = PhysicalFS::new("test/folder_b");

        let roc_fs = RocFS::new(vec![Box::new(folder_a), Box::new(folder_b)]);
        let root = read_directory(&roc_fs, "/");

        itertools::assert_equal(root.keys(), vec!["file_a", "file_b"]);
        itertools::assert_equal(root.values(), vec![&Metadata::file(6), &Metadata::file(6)])
    }

    #[test]
    fn read_dir_missing_folder() {
        let folder_a = PhysicalFS::new("test/folder_a");
        let folder_c = PhysicalFS::new("test/folder_c");

        let roc_fs = RocFS::new(vec![Box::new(folder_a), Box::new(folder_c)]);
        let root = read_directory(&roc_fs, "/");

        itertools::assert_equal(root.keys(), vec!["file_a"]);
        itertools::assert_equal(root.values(), vec![&Metadata::file(6)])
    }

    #[test]
    fn read_dir_missing_folders() {
        let folder_c = PhysicalFS::new("test/folder_c");
        let folder_d = PhysicalFS::new("test/folder_d");

        let roc_fs = RocFS::new(vec![Box::new(folder_c), Box::new(folder_d)]);
        let root = read_directory(&roc_fs, "/");

        assert!(root.is_empty());
    }

    #[test]
    fn open_file_happy_case() {
        let folder_a = PhysicalFS::new("test/folder_a");
        let folder_b = PhysicalFS::new("test/folder_b");

        let roc_fs = RocFS::new(vec![Box::new(folder_a), Box::new(folder_b)]);

        let file_a = roc_fs
            .open_file("/file_a")
            .unwrap()
            .read_into_string()
            .unwrap();

        let file_b = roc_fs
            .open_file("/file_b")
            .unwrap()
            .read_into_string()
            .unwrap();

        assert_eq!(file_a, "file a");
        assert_eq!(file_b, "file b");
    }

    #[test]
    fn open_file_not_found() {
        let roc_fs = RocFS::new(vec![]);

        let open_res = roc_fs.open_file("abc");

        assert!(open_res.is_err());
        assert_eq!(open_res.err().unwrap().kind(), ErrorKind::NotFound);
    }
}
