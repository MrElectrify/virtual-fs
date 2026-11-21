use crate::file::{DirEntry, File, Metadata, OpenOptions};
use crate::util::{invalid_input, not_found, not_supported};
use crate::FileSystem;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::fmt::Debug;
use std::io;
use std::io::{Cursor, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use zip::result::{ZipError, ZipResult};
use zip::ZipArchive;

/// The default maximum size of a single decompressed file, 256 MiB.
pub const DEFAULT_MAX_FILE_SIZE: u64 = 256 * 1024 * 1024;

/// The maximum number of bytes reserved up front based on the size declared by an entry.
const MAX_PREALLOCATION: u64 = 16 * 1024 * 1024;

/// A virtual FileSystem backed by a ZIP file. Only supports read operations for now.
///
/// Entry names are normalized when the archive is mounted: both `/` and `\` are treated as
/// separators, empty and `.` components are dropped, and `..` components are resolved without
/// ever escaping the root of the archive. Lookups are case-insensitive.
#[derive(Debug)]
pub struct ZipFS<R: Read + Seek> {
    zip_file: Mutex<ZipArchive<R>>,
    index: Index,
    max_file_size: u64,
}

/// An indexed entry of the archive.
#[derive(Debug)]
struct Entry {
    /// The cased file name of the entry, as found in the archive.
    name: String,
    node: Node,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Node {
    /// A file, located at `index` in the archive, with a declared uncompressed `size`.
    File { index: usize, size: u64 },
    /// A directory, either explicitly stored in the archive or implied by a file path.
    Directory,
}

impl Node {
    fn metadata(&self) -> Metadata {
        match *self {
            Node::File { size, .. } => Metadata::file(size),
            Node::Directory => Metadata::directory(),
        }
    }
}

/// The index of all files and directories in the archive.
#[derive(Debug)]
struct Index {
    /// All entries, keyed by normalized lowercase path. The root is `""`.
    entries: HashMap<String, Entry>,
    /// The normalized lowercase paths of the children of each directory.
    children: HashMap<String, Vec<String>>,
}

impl Index {
    fn build<R: Read + Seek>(zip_file: &mut ZipArchive<R>) -> ZipResult<Self> {
        let mut index = Self {
            entries: HashMap::from([(
                String::new(),
                Entry {
                    name: String::new(),
                    node: Node::Directory,
                },
            )]),
            children: HashMap::from([(String::new(), Vec::new())]),
        };

        for i in 0..zip_file.len() {
            // read the raw entry, so that encrypted entries do not prevent mounting
            let entry = zip_file.by_index_raw(i)?;
            let name = entry.name().to_owned();
            let size = entry.size();
            drop(entry);

            // directory entries end with a separator
            let is_dir = name.ends_with(['/', '\\']);
            let components = normalized_components(&name);
            let mut path = String::new();
            for (n, component) in components.iter().enumerate() {
                let parent = path.clone();
                if !path.is_empty() {
                    path.push('/');
                }
                path.push_str(&component.to_lowercase());

                let node = if n + 1 == components.len() && !is_dir {
                    Node::File { index: i, size }
                } else {
                    Node::Directory
                };
                index.insert(parent, path.clone(), component, node)?;
            }
        }

        Ok(index)
    }

    fn insert(&mut self, parent: String, path: String, name: &str, node: Node) -> ZipResult<()> {
        if let Some(existing) = self.entries.get(&path) {
            // directories may be implied by multiple entries, but anything else would make one
            // entry shadow another
            return if existing.node == Node::Directory && node == Node::Directory {
                Ok(())
            } else {
                Err(ZipError::InvalidArchive(
                    "Conflicting entries in zip archive",
                ))
            };
        }

        self.children.entry(parent).or_default().push(path.clone());
        if node == Node::Directory {
            self.children.entry(path.clone()).or_default();
        }
        self.entries.insert(
            path,
            Entry {
                name: name.to_owned(),
                node,
            },
        );
        Ok(())
    }

    fn get(&self, path: &str) -> crate::Result<&Entry> {
        self.entries.get(&lookup_key(path)).ok_or_else(not_found)
    }
}

/// Splits a path into its normalized components. Both `/` and `\` are treated as separators,
/// empty and `.` components are dropped and `..` removes the previous component, but never
/// escapes the root.
fn normalized_components(path: &str) -> Vec<&str> {
    let mut components = Vec::new();
    for component in path.split(['/', '\\']) {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            component => components.push(component),
        }
    }
    components
}

/// Returns the lookup key of a path: its normalized components, lowercased and joined by `/`.
fn lookup_key(path: &str) -> String {
    normalized_components(path).join("/").to_lowercase()
}

impl<R: Read + Seek> ZipFS<R> {
    /// Mounts a ZIP file onto the local filesystem. Files larger than [`DEFAULT_MAX_FILE_SIZE`]
    /// cannot be opened, see [`ZipFS::with_max_file_size`].
    pub fn new(zip_file: R) -> ZipResult<Self> {
        Self::with_max_file_size(zip_file, DEFAULT_MAX_FILE_SIZE)
    }

    /// Mounts a ZIP file onto the local filesystem.
    ///
    /// Opening a file decompresses it into memory. To guard against decompression bombs, files
    /// that declare or decompress to more than `max_file_size` bytes fail to open with
    /// [`ErrorKind::InvalidData`].
    pub fn with_max_file_size(zip_file: R, max_file_size: u64) -> ZipResult<Self> {
        let mut zip_file = ZipArchive::new(zip_file)?;
        let index = Index::build(&mut zip_file)?;

        Ok(Self {
            zip_file: Mutex::new(zip_file),
            index,
            max_file_size,
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
}

impl<R: Read + Seek> FileSystem for ZipFS<R> {
    fn create_dir(&self, _path: &str) -> crate::Result<()> {
        Err(not_supported())
    }

    fn metadata(&self, path: &str) -> crate::Result<Metadata> {
        Ok(self.index.get(path)?.node.metadata())
    }

    fn open_file_options(&self, path: &str, options: &OpenOptions) -> crate::Result<Box<dyn File>> {
        // ensure we only want to read
        if !options.read || options.write {
            return Err(not_supported());
        }

        let (index, size) = match self.index.get(path)?.node {
            Node::File { index, size } => (index, size),
            Node::Directory => return Err(invalid_input("Is a directory")),
        };

        // open the file and read into a readable buffer, never trusting the declared size
        let too_large = || io::Error::new(ErrorKind::InvalidData, "File exceeds the maximum size");
        if size > self.max_file_size {
            return Err(too_large());
        }

        let mut zip_file = self.zip_file.lock();
        let entry = Self::convert_error(zip_file.by_index(index))?;
        let mut contents = Vec::with_capacity(size.min(MAX_PREALLOCATION) as usize);
        entry
            .take(self.max_file_size.saturating_add(1))
            .read_to_end(&mut contents)?;
        if contents.len() as u64 > self.max_file_size {
            return Err(too_large());
        }

        Ok(Box::new(ZipFileContents {
            inner: Cursor::new(contents),
        }))
    }

    fn read_dir(
        &self,
        path: &str,
    ) -> crate::Result<Box<dyn Iterator<Item = crate::Result<DirEntry>>>> {
        let key = lookup_key(path);
        let children = self.index.children.get(&key).ok_or_else(|| {
            if self.index.entries.contains_key(&key) {
                invalid_input("Not a directory")
            } else {
                not_found()
            }
        })?;

        let entries = children
            .iter()
            .map(|child| {
                let entry = &self.index.entries[child];
                Ok(DirEntry {
                    path: PathBuf::from(&entry.name),
                    metadata: entry.node.metadata(),
                })
            })
            .collect::<Vec<_>>();

        Ok(Box::new(entries.into_iter()))
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
    use std::io::{Cursor, ErrorKind, Read, Seek, Write};
    use zip::result::ZipError;
    use zip::write::{FileOptions, ZipWriter};

    fn read_directory<R: Read + Seek>(
        fs: &ZipFS<R>,
        path: &str,
    ) -> crate::Result<BTreeMap<String, Metadata>> {
        Ok(fs
            .read_dir(path)?
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.path.to_str().unwrap().to_owned(), entry.metadata)
            })
            .collect::<BTreeMap<_, _>>())
    }

    fn read_string<R: Read + Seek>(fs: &ZipFS<R>, path: &str) -> crate::Result<String> {
        fs.open_file(path)?.read_into_string()
    }

    fn zip_fs() -> ZipFS<File> {
        ZipFS::new(File::open("test/deep_fs.zip").unwrap()).unwrap()
    }

    /// Builds an in-memory ZIP archive with the given directory and file entries.
    fn build_zip(dirs: &[&str], files: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for dir in dirs {
            writer.add_directory(*dir, FileOptions::default()).unwrap();
        }
        for (name, contents) in files {
            writer.start_file(*name, FileOptions::default()).unwrap();
            writer.write_all(contents.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn memory_zip_fs(dirs: &[&str], files: &[(&str, &str)]) -> ZipFS<Cursor<Vec<u8>>> {
        ZipFS::new(Cursor::new(build_zip(dirs, files))).unwrap()
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
    fn read_dir_errors() {
        let fs = zip_fs();

        let err = read_directory(&fs, "file").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);

        let err = read_directory(&fs, "not_a_real_path").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn mixed_case_directories() {
        let fs = memory_zip_fs(&[], &[("Folder/Sub/File.txt", "contents")]);

        for path in ["Folder", "folder", "FOLDER"] {
            let listing = read_directory(&fs, path).unwrap();
            itertools::assert_equal(listing.keys(), vec!["Sub"]);
            assert!(listing["Sub"].is_directory());
        }

        let listing = read_directory(&fs, "folder/sub").unwrap();
        itertools::assert_equal(listing.keys(), vec!["File.txt"]);
        assert_eq!(read_string(&fs, "FOLDER/SUB/FILE.TXT").unwrap(), "contents");
    }

    #[test]
    fn non_canonical_names() {
        for name in [
            "./dir/file.txt",
            "/dir/file.txt",
            "../dir/file.txt",
            "dir\\file.txt",
            "dir//file.txt",
            "dir/./other/../file.txt",
        ] {
            let fs = memory_zip_fs(&[], &[(name, "data")]);

            let root = read_directory(&fs, "").unwrap();
            itertools::assert_equal(root.keys(), vec!["dir"]);
            assert!(root["dir"].is_directory(), "{name}");

            let dir = read_directory(&fs, "dir").unwrap();
            itertools::assert_equal(dir.keys(), vec!["file.txt"]);
            assert_eq!(read_string(&fs, "dir/file.txt").unwrap(), "data", "{name}");
        }
    }

    #[test]
    fn empty_directories() {
        let fs = memory_zip_fs(&["empty/", "nested/empty/"], &[]);

        let root = read_directory(&fs, "").unwrap();
        itertools::assert_equal(root.keys(), vec!["empty", "nested"]);
        assert!(root.values().all(Metadata::is_directory));

        assert!(fs.metadata("empty").unwrap().is_directory());
        assert!(fs.metadata("nested/empty").unwrap().is_directory());
        assert!(read_directory(&fs, "empty").unwrap().is_empty());
        assert!(read_directory(&fs, "nested/empty").unwrap().is_empty());
    }

    #[test]
    fn conflicting_entries_are_rejected() {
        for files in [
            &[("a.txt", "first"), ("A.TXT", "second")][..],
            &[("a.txt", "first"), ("./a.txt", "second")][..],
            &[("dir/a.txt", "first"), ("../dir/a.txt", "second")][..],
            &[("a", "file"), ("a/b", "file below a file")][..],
        ] {
            let err = ZipFS::new(Cursor::new(build_zip(&[], files))).unwrap_err();
            assert!(
                matches!(err, ZipError::InvalidArchive(_)),
                "{files:?}: {err:?}"
            );
        }

        let err = ZipFS::new(Cursor::new(build_zip(&["a/"], &[("A", "file")]))).unwrap_err();
        assert!(matches!(err, ZipError::InvalidArchive(_)), "{err:?}");

        // directories implied by several entries are fine
        let fs = memory_zip_fs(&["dir/", "DIR/"], &[("dir/a", "a"), ("Dir/b", "b")]);
        itertools::assert_equal(read_directory(&fs, "dir").unwrap().keys(), vec!["a", "b"]);
    }

    /// Returns the offset of the first central directory file header.
    fn central_directory_header(zip: &[u8]) -> usize {
        zip.windows(4)
            .position(|window| window == [0x50, 0x4b, 0x01, 0x02])
            .unwrap()
    }

    #[test]
    fn file_size_limit() {
        let zip = build_zip(&[], &[("small", "1234"), ("large", "12345")]);
        let fs = ZipFS::with_max_file_size(Cursor::new(zip), 4).unwrap();

        assert_eq!(read_string(&fs, "small").unwrap(), "1234");
        assert_eq!(
            fs.open_file("large").err().unwrap().kind(),
            ErrorKind::InvalidData
        );
        // metadata is still available
        assert_eq!(fs.metadata("large").unwrap().len(), 5);
    }

    #[test]
    fn decompression_bomb_is_rejected() {
        // 64 MiB of zeros compress to a few dozen KiB
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("bomb", FileOptions::default()).unwrap();
        let chunk = vec![0u8; 1024 * 1024];
        for _ in 0..64 {
            writer.write_all(&chunk).unwrap();
        }
        let zip = writer.finish().unwrap().into_inner();
        assert!(zip.len() < 1024 * 1024);

        let fs = ZipFS::with_max_file_size(Cursor::new(zip), 1024 * 1024).unwrap();
        assert_eq!(
            fs.open_file("bomb").err().unwrap().kind(),
            ErrorKind::InvalidData
        );
    }

    #[test]
    fn declared_size_is_not_trusted() {
        let stored = FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("liar", stored).unwrap();
        writer.write_all(b"hello world").unwrap();
        let zip = writer.finish().unwrap().into_inner();

        // an absurdly large declared size fails without allocating it
        let mut huge = zip.clone();
        let header = central_directory_header(&huge);
        huge[header + 24..header + 28].copy_from_slice(&u32::MAX.to_le_bytes());
        let fs = ZipFS::new(Cursor::new(huge)).unwrap();
        assert_eq!(
            fs.open_file("liar").err().unwrap().kind(),
            ErrorKind::InvalidData
        );

        // a declared size below the limit does not allow reading past the limit
        let mut tiny = zip;
        let header = central_directory_header(&tiny);
        tiny[header + 24..header + 28].copy_from_slice(&1u32.to_le_bytes());
        let fs = ZipFS::with_max_file_size(Cursor::new(tiny), 4).unwrap();
        assert_eq!(
            fs.open_file("liar").err().unwrap().kind(),
            ErrorKind::InvalidData
        );
    }

    #[test]
    fn open_directory_fails() {
        let fs = zip_fs();

        assert_eq!(
            fs.open_file("folder").err().unwrap().kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn encrypted_entries_do_not_prevent_mounting() {
        let mut zip = build_zip(&[], &[("secret", "hidden"), ("public", "visible")]);

        // set the "encrypted" flag on the central directory header of the first entry
        let header = zip
            .windows(4)
            .position(|window| window == [0x50, 0x4b, 0x01, 0x02])
            .unwrap();
        zip[header + 8] |= 1;

        let fs = ZipFS::new(Cursor::new(zip)).unwrap();
        assert_eq!(read_string(&fs, "public").unwrap(), "visible");
        assert_eq!(
            fs.open_file("secret").err().unwrap().kind(),
            ErrorKind::Unsupported
        );
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
        assert!(fs.exists("FiLe").unwrap());
        assert!(!fs.exists("no_file").unwrap());
        assert!(fs.exists("folder").unwrap());
        assert!(fs.exists("folDeR").unwrap());
        assert!(fs.exists("folder/and/it").unwrap());
        assert!(fs.exists("folder/anD/iT").unwrap());
        assert!(fs.exists("folder/and/it/desc").unwrap());
        assert!(!fs.exists("folder/and/it/does/not").unwrap());
        assert!(fs.exists("///test/something_else/../../file").unwrap());
        assert!(fs.exists("///test/something_elsE/../../file").unwrap());
    }
}
