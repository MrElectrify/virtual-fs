use crate::file::{File, Metadata, OpenOptions};
use crate::util::{invalid_input, not_supported};
use enumflags2::{bitflags, BitFlags};
use parking_lot::MutexGuard;
use std::io::{Read, Seek, SeekFrom, Write};
use std::{io, mem};

/// The file open mode.
#[bitflags]
#[derive(Debug, Copy, Clone)]
#[repr(u8)]
pub enum FileMode {
    Read,
    Write,
}

impl FileMode {
    /// Collections file options from the associated open options.
    ///
    /// # Arguments
    /// `open_options`: The open options.  
    pub fn from_options(open_options: &OpenOptions) -> BitFlags<Self> {
        let mut mode = BitFlags::empty();
        if open_options.read {
            mode.insert(FileMode::Read);
        }
        if open_options.write {
            mode.insert(FileMode::Write);
        }

        mode
    }
}

pub struct FileHandle {
    contents: MutexGuard<'static, Vec<u8>>,
    // safety: mutex must be defined after `contents` so that `Drop` will drop the mutex guard before the mutex
    _mutex: super::File,
    pos: usize,
    mode: BitFlags<FileMode>,
}

impl FileHandle {
    /// Creates a new file handle with the given content mutex and mode.
    ///
    /// # Arguments
    /// `contents_mutex`: The mutex surrounding the contents. This prevents multiple concurrent file accesses.  
    /// `mode`: The file open mode.  
    pub fn new(contents_mutex: super::File, mode: BitFlags<FileMode>) -> Self {
        // safety: as long as this struct is alive, `contents` will be alive.
        let contents = contents_mutex.lock();

        Self {
            contents: unsafe { mem::transmute(contents) },
            _mutex: contents_mutex,
            pos: 0,
            mode,
        }
    }

    /// Clear the contents of the file.
    pub fn clear(&mut self) {
        self.contents.clear()
    }

    /// Return the remaining file contents as a slice.
    fn remaining_slice(&self) -> &[u8] {
        let start_pos = self.pos.min(self.contents.len());
        &self.contents[start_pos..]
    }

    /// Checks to ensure that the required mode is active.
    fn check_mode(mode: bool) -> io::Result<()> {
        if mode {
            Ok(())
        } else {
            Err(not_supported())
        }
    }
}

impl Read for FileHandle {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Self::check_mode(self.mode.contains(FileMode::Read))?;

        let mut remaining_slice = self.remaining_slice();
        let n = remaining_slice.read(buf)?;
        self.pos += n;

        Ok(n)
    }
}

impl Seek for FileHandle {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let (base_pos, offset) = match pos {
            SeekFrom::Start(n) => {
                self.pos = n as usize;
                return Ok(n);
            }
            SeekFrom::Current(n) => (self.pos as u64, n),
            SeekFrom::End(n) => (self.contents.len() as u64, n),
        };

        if let Some(n) = base_pos.checked_add_signed(offset) {
            self.pos = n as usize;
            Ok(n)
        } else {
            Err(invalid_input(
                "Invalid seek to a negative or overflowing position",
            ))
        }
    }
}

impl Write for FileHandle {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Self::check_mode(self.mode.contains(FileMode::Write))?;

        let pos = self.pos.min(self.contents.len());
        let needed_len = pos.saturating_add(buf.len());

        if needed_len > self.contents.len() {
            // we could write this with some unsafe uninit stuff, but meh
            self.contents.resize(needed_len, 0);
        }

        self.contents[pos..needed_len].copy_from_slice(buf);

        Ok(needed_len - pos)
    }

    fn flush(&mut self) -> io::Result<()> {
        Self::check_mode(self.mode.contains(FileMode::Write))?;

        // there's nothing to flush
        Ok(())
    }
}

impl File for FileHandle {
    fn metadata(&self) -> crate::Result<Metadata> {
        Ok(Metadata::file(self.contents.len() as u64))
    }
}
