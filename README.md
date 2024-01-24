# Virtual Filesystems for Rust
This crate defines and implements various virtual filesystems for Rust. It's loosely inspired by the `vfs` crate with
a focus on conformity with `std`.

`virtual-fs` has the following FileSystems implemented out of the box:
- `PhysicalFS`: A read-write physical filesystem mounted at a directory. Path traversal outside the root is permitted.
- `SandboxedPhysicalFS`: A read-write physical filesystem that guards against traversal through backtracking and symbolic link
traversal.
- `MemoryFS`: A read-write in-memory filesystem.
- `RocFS`: A "read-only collection" filesystem. This filesystem is similar to `OverlayFS`, but is read-only. This
filesystem searches filesystems in mount-order for files, allowing multiple filesystems to be mounted at once.
- `MountableFS`: A read-write filesystem that supports mounting other filesystems at given paths.
- `ZipFS`: A read-only filesystem that mounts a ZIP archive, backed by the `zip` crate.
- `TarFS` A read-only filesystem that mounts a Tarball, backed by the `tar` crate.