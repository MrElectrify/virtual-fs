use crate::util::make_relative;
use std::io;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Resolves paths to their respective host paths.
pub trait PathResolver {
    /// Resolves `path` to a suitable host path rooted at `root`.
    fn resolve_path(root: &Path, path: &str) -> crate::Result<PathBuf>;
}

/// A resolver that ensures that paths have not been traversed, either through backtracking or symbolic links.
pub struct SandboxedPathResolver {}
impl PathResolver for SandboxedPathResolver {
    fn resolve_path(root: &Path, path: &str) -> crate::Result<PathBuf> {
        // root is already normalized by `PhysicalFSImpl`
        let root = root.canonicalize()?;
        let host_path = root.join(make_relative(path)).canonicalize()?;

        if !host_path.starts_with(root) {
            return Err(io::Error::new(
                ErrorKind::PermissionDenied,
                "Traversal prevented",
            ));
        }

        Ok(host_path)
    }
}

/// An unrestricted path resolver that simply appends the desired path to the root without checking for bounds.
pub struct UnrestrictedPathResolver {}
impl PathResolver for UnrestrictedPathResolver {
    fn resolve_path(root: &Path, path: &str) -> crate::Result<PathBuf> {
        Ok(root.join(make_relative(path)))
    }
}

#[cfg(test)]
mod test {
    use crate::physical_fs::path_resolver::{
        PathResolver, SandboxedPathResolver, UnrestrictedPathResolver,
    };
    use std::path::Path;

    #[test]
    fn sandboxed_resolver() {
        assert_eq!(
            SandboxedPathResolver::resolve_path(Path::new("test/a/b/c"), "/d/e/f").unwrap(),
            Path::new("test/a/b/c/d/e/f").canonicalize().unwrap()
        );
        assert_eq!(
            SandboxedPathResolver::resolve_path(Path::new("test/a/b/c"), "\\d//\\e/f").unwrap(),
            Path::new("test/a/b/c/d/e/f").canonicalize().unwrap()
        );
        assert_eq!(
            SandboxedPathResolver::resolve_path(Path::new("test/a/b/c"), "./d/e/f").unwrap(),
            Path::new("test/a/b/c/d/e/f").canonicalize().unwrap()
        );
        assert_eq!(
            SandboxedPathResolver::resolve_path(Path::new("test/a/b/c"), "d/e/g/../f").unwrap(),
            Path::new("test/a/b/c/d/e/f").canonicalize().unwrap()
        );
        assert_eq!(
            SandboxedPathResolver::resolve_path(Path::new("test/a/b/c"), "../../b/c/d").unwrap(),
            Path::new("test/a/b/c/d").canonicalize().unwrap()
        );
        // traversal
        assert!(SandboxedPathResolver::resolve_path(
            Path::new("test/a/b/c"),
            "d/e/f/g/../../../../.."
        )
        .is_err());
        // symlink
        assert!(SandboxedPathResolver::resolve_path(Path::new("test"), "virtual-fs").is_err());
    }

    #[test]
    fn unrestricted_resolver() {
        assert_eq!(
            UnrestrictedPathResolver::resolve_path(Path::new("/a/b/c"), "/d/e/f").unwrap(),
            Path::new("/a/b/c/d/e/f")
        );
        assert_eq!(
            UnrestrictedPathResolver::resolve_path(Path::new("/a/b/c"), "./d/e/f").unwrap(),
            Path::new("/a/b/c/d/e/f")
        );
        assert_eq!(
            UnrestrictedPathResolver::resolve_path(Path::new("/a/b/c"), "../d/e/f").unwrap(),
            Path::new("/a/b/c/../d/e/f")
        );
    }
}
