use crate::file::Metadata;
use crate::memory_fs::File;
use crate::tree::Entry;

impl From<&Entry<File>> for Metadata {
    fn from(value: &Entry<File>) -> Self {
        match value {
            Entry::Directory(_) => Self::directory(),
            Entry::UserData(file) => Self::file(file.lock().len() as u64),
        }
    }
}
