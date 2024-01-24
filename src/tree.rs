use crate::util::{component_iter, invalid_path, make_relative, normalize_path, not_found};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A directory in tree-based filesystem.
pub type Directory<T> = HashMap<String, Entry<T>>;

/// A directory node in the file tree.
pub enum Entry<T> {
    Directory(HashMap<String, Entry<T>>),
    UserData(T),
}

impl<T> Default for Entry<T> {
    fn default() -> Self {
        Self::Directory(HashMap::default())
    }
}

/// A tree-based filesystem with directories and other data.
pub struct FilesystemTree<T> {
    root: Mutex<Entry<T>>,
}

impl<T> FilesystemTree<T> {
    /// Creates all directories specified in `path`, including the trailing path. Calls `f` with the resulting
    /// directory on success.
    ///
    /// # Arguments
    /// `path`: The path to create all of the directories for.  
    /// `f`: The function.  
    pub fn create_dir_all<R, P: AsRef<Path>, F: FnOnce(&mut Directory<T>) -> R>(
        &self,
        path: P,
        f: F,
    ) -> crate::Result<R> {
        // specialize this method so we don't turn this into O(n^2) searching for each subcomponent
        let mut entry = self.root.lock();
        let mut entry = &mut *entry;
        for component in component_iter(&normalize_and_relativize(path)) {
            let Entry::Directory(dir) = entry else {
                return Err(not_found());
            };

            entry = dir
                .entry(component.to_owned())
                .or_insert_with(|| Entry::Directory(HashMap::default()));
        }

        // make sure the last entry was also a directory
        if let Entry::Directory(dir) = entry {
            Ok(f(dir))
        } else {
            Err(not_found())
        }
    }

    /// Calls `f` with the directory at the specified path, only if it is located.
    ///
    /// # Arguments
    /// `path`: The directory to fetch the entry for.  
    pub fn with_directory<R, P: AsRef<Path>, F: FnOnce(&mut Directory<T>) -> R>(
        &self,
        path: P,
        f: F,
    ) -> crate::Result<R> {
        self.with_entry(path, |entry| entry.map(f).map_err(|_| not_found()))
    }

    /// Calls `f` with the entry at `path`, or the last found entry and remaining path.
    ///
    /// # Arguments
    /// `normalized_path`: The normalized path as
    pub fn with_entry<
        R,
        P: AsRef<Path>,
        F: FnOnce(Result<&mut Directory<T>, (&mut T, &Path)>) -> crate::Result<R>,
    >(
        &self,
        path: P,
        f: F,
    ) -> crate::Result<R> {
        // normalize the path
        let normalized_path = normalize_and_relativize(path);
        let mut normalized_path = normalized_path.as_path();

        // iterate through each component until we hit a filesystem
        let mut entry = self.root.lock();
        let mut entry = &mut *entry;
        for component in component_iter(normalized_path) {
            match entry {
                Entry::Directory(directory) => {
                    normalized_path = normalized_path
                        .strip_prefix(format!("{component}/"))
                        .map_err(|_| invalid_path())?;

                    // traverse into the directory
                    entry = directory.get_mut(component).ok_or_else(not_found)?;
                }
                Entry::UserData(ud) => {
                    // there can't be a valid component after resolving a file
                    return f(Err((ud, normalized_path)));
                }
            }
        }

        // entry has to be a directory unless the root is a filesystem
        match entry {
            Entry::Directory(dir) => f(Ok(dir)),
            Entry::UserData(ud) => f(Err((ud, normalized_path))),
        }
    }
}

impl<T> Default for FilesystemTree<T> {
    fn default() -> Self {
        Self {
            root: Mutex::default(),
        }
    }
}

/// Normalizes a path by making it relative and resolving any backtracking.
pub(crate) fn normalize_and_relativize<P: AsRef<Path>>(p: P) -> PathBuf {
    // treat every path as relative from the root
    normalize_path(make_relative(p))
}
