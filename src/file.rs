use std::fs;
use std::io::{Read, Seek, Write};
use std::path::PathBuf;

/// The type of a file.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum FileType {
    /// A directory.
    Directory,
    /// A file.
    File,
    /// The file type is unknown or unsupported.
    Unknown,
}

impl From<fs::FileType> for FileType {
    fn from(value: fs::FileType) -> Self {
        if value.is_dir() {
            Self::Directory
        } else if value.is_file() {
            Self::File
        } else {
            Self::Unknown
        }
    }
}

/// A directory entry.
#[derive(Debug, Clone)]
pub struct DirEntry {
    /// The path to the file.
    pub path: PathBuf,
    /// Metadata about the file.
    pub metadata: Metadata,
}

impl DirEntry {
    /// Returns true if the entry is a directory.
    pub fn is_directory(&self) -> bool {
        self.metadata.is_directory()
    }

    /// Returns true if the entry is a file.
    pub fn is_file(&self) -> bool {
        self.metadata.is_directory()
    }

    /// Returns the length of the file, in bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> u64 {
        self.metadata.len()
    }
}

/// Metadata about a file.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Metadata {
    /// True if the entry is a directory.
    pub file_type: FileType,
    /// The length of the file.
    pub len: u64,
}

impl Metadata {
    /// Returns metadata for a directory
    pub fn directory() -> Self {
        Self {
            file_type: FileType::Directory,
            len: 0,
        }
    }

    /// Returns metadata for a file.
    pub fn file(len: u64) -> Self {
        Self {
            file_type: FileType::File,
            len,
        }
    }

    /// Returns true if the entry is a directory.
    pub fn is_directory(&self) -> bool {
        self.file_type == FileType::Directory
    }

    /// Returns true if the entry is a file.
    pub fn is_file(&self) -> bool {
        self.file_type == FileType::File
    }

    /// Returns the length of the file, in bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> u64 {
        self.len
    }
}

impl From<fs::Metadata> for Metadata {
    fn from(value: fs::Metadata) -> Self {
        Self {
            file_type: value.file_type().into(),
            len: value.len(),
        }
    }
}

/// Options for opening a file. The default mode is read-only.
#[derive(Debug)]
pub struct OpenOptions {
    /// True if the file should be able to be appended to.
    pub append: bool,
    /// True if the file should be created if not present.
    pub create: bool,
    /// True if the file should be able to be read.
    pub read: bool,
    /// True if the file should be truncated.
    pub truncate: bool,
    /// True if the file should be written to.
    pub write: bool,
}

impl From<&OpenOptions> for fs::OpenOptions {
    fn from(value: &OpenOptions) -> Self {
        Self::new()
            .create(value.create)
            .append(value.append)
            .truncate(value.truncate)
            .read(value.read)
            .clone()
    }
}

impl OpenOptions {
    /// # Arguments
    /// `append`: If true, the file should be opened with the cursor set to the end of the file,
    /// rather than overwriting the file contents. Note that setting this to true will implicitly
    /// enable writing.  
    pub fn append(mut self, append: bool) -> Self {
        if append {
            self.write = true;
        }
        self.append = append;
        self.truncate = !append;
        self
    }

    /// # Arguments
    /// `append`: If true, the file should be created if it does not exist. Note that setting this
    /// to true will implicitly enable writing.  
    pub fn create(mut self, create: bool) -> Self {
        if create {
            self.write = true;
        }
        self.create = true;
        self
    }

    /// # Arguments
    /// `read`: If true, the file should be able to be read in entirety.  
    pub fn read(mut self, read: bool) -> Self {
        self.read = read;
        self
    }

    /// # Arguments
    /// `truncate`: If true, the file should be opened with the cursor set to the beginning of the
    /// file, overwriting all contents. Note that setting this to true will implicitly enable
    /// writing.  
    pub fn truncate(mut self, truncate: bool) -> Self {
        if truncate {
            self.write = true;
        }
        self.append = !truncate;
        self.truncate = truncate;
        self
    }

    /// # Arguments
    /// `write`: If true, the file should be able to be written. By default, this will truncate
    /// the contents of the file, unless `append` is set.
    pub fn write(mut self, write: bool) -> Self {
        self.write = write;
        self
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            append: false,
            create: false,
            read: true,
            truncate: false,
            write: false,
        }
    }
}

/// A file that can be read.
pub trait File: Read + Write + Seek {
    /// Returns the directory entry for the file.
    fn metadata(&self) -> crate::Result<Metadata>;

    /// Reads a file into a vector.
    fn read_into_vec(&mut self) -> crate::Result<Vec<u8>> {
        let mut vec = Vec::with_capacity(self.metadata()?.len() as usize);
        self.read_to_end(&mut vec)?;
        Ok(vec)
    }

    /// Reads a file into a string.
    fn read_into_string(&mut self) -> crate::Result<String> {
        let mut str = String::with_capacity(self.metadata()?.len() as usize);
        self.read_to_string(&mut str)?;
        Ok(str)
    }
}
