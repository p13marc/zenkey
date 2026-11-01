//! Runs (core §4.3, #976): an owner's ordering epochs, and the record a
//! 16-bit run is counted in.
//!
//! ```text
//! a new instance                     → a new run (§1.5, §4.3)
//!   with a RunStore                  → its next 16-bit counter, durable before any write under it
//!   with none, a spent counter, or a
//!   redundancy.v1 candidate          → a random 32-bit run, drawn again while it equals one used
//! within an instance, before the state and event seq would pass 2^24 − 1,
//!   or past STREAM_KEYS stream keys  → a new run, the descriptor put stating it first
//! ```
//!
//! A run is never reused (core §4.3's MUST NOT): a counter only moves
//! forward, its record made durable before the run is handed out, and a
//! record that cannot be read is read as spent, never reset, since a reset
//! would issue its runs again. An owner that cannot keep its record with
//! it when it moves (core §1.5: a move changes no key) keeps none, and
//! draws 32-bit runs.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// Where an owner keeps the counter its 16-bit runs come from (core
/// §4.3): one per service, kept with it wherever it runs.
pub trait RunStore: Send + Sync + 'static {
    /// The next 16-bit run, durable before it returns: a write under it
    /// may follow at once. `Ok(None)` once the counter is spent, at
    /// 0xFFFF, or when the record cannot be read: the owner draws 32-bit
    /// runs from then on. `Err` is an I/O failure, which the owner logs
    /// and answers with a 32-bit run as well, never a refusal: a 32-bit
    /// run never equals a 16-bit one.
    ///
    /// # Errors
    ///
    /// The record could not be written durably.
    fn next(&self) -> io::Result<Option<u16>>;
}

/// A [`RunStore`] in a file: 2 bytes, big-endian, the last run issued.
///
/// - **Absent:** none issued yet, so the first run is 0.
/// - **Each run** is written to `<path>.tmp`, synced, renamed over the
///   record, and the directory synced, before [`RunStore::next`] returns
///   it.
/// - **Spent:** a record at 0xFFFF gives `None`, and is left as it is.
/// - **Unreadable** (not 2 bytes): read as spent, `None` and a warning. It
///   is never reset, since a reset would issue its runs again.
#[derive(Debug, Clone)]
pub struct FileRunStore {
    path: PathBuf,
}

impl FileRunStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Its record.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The last run the record holds: `Ok(None)` with no record, and
    /// `Err` with one that is not 2 bytes, or cannot be read.
    ///
    /// # Errors
    ///
    /// The record is not 2 bytes ([`io::ErrorKind::InvalidData`]), or could
    /// not be read.
    pub fn last(&self) -> io::Result<Option<u16>> {
        match std::fs::read(&self.path) {
            Ok(bytes) => match <[u8; 2]>::try_from(bytes.as_slice()) {
                Ok(b) => Ok(Some(u16::from_be_bytes(b))),
                Err(_) => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} bytes, not the 2 of a run record", bytes.len()),
                )),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write(&self, run: u16) -> io::Result<()> {
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&run.to_be_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        let dir = match self.path.parent() {
            Some(d) if !d.as_os_str().is_empty() => d,
            _ => Path::new("."),
        };
        std::fs::File::open(dir)?.sync_all()
    }
}

impl RunStore for FileRunStore {
    fn next(&self) -> io::Result<Option<u16>> {
        let next = match self.last() {
            Ok(None) => 0,
            Ok(Some(u16::MAX)) => return Ok(None),
            Ok(Some(n)) => n + 1,
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                tracing::warn!(
                    record = %self.path.display(),
                    "the run record cannot be read ({e}): it is read as spent, never reset, so \
                     this owner draws 32-bit runs (core §4.3)"
                );
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        self.write(next)?;
        Ok(Some(next))
    }
}

#[cfg(test)]
mod tests {
    use super::{FileRunStore, RunStore};

    #[test]
    fn a_file_counts_from_zero_and_is_spent_at_ffff() {
        let dir = tempfile::tempdir().unwrap();
        let s = FileRunStore::new(dir.path().join("run"));
        assert_eq!(s.last().unwrap(), None);
        assert_eq!(s.next().unwrap(), Some(0));
        assert_eq!(s.next().unwrap(), Some(1));
        assert_eq!(std::fs::read(s.path()).unwrap(), [0, 1]);
        assert_eq!(FileRunStore::new(s.path()).next().unwrap(), Some(2));
        std::fs::write(s.path(), [0xff, 0xfe]).unwrap();
        assert_eq!(s.next().unwrap(), Some(0xffff));
        assert_eq!(s.next().unwrap(), None, "spent");
        assert_eq!(
            std::fs::read(s.path()).unwrap(),
            [0xff, 0xff],
            "left as it is"
        );
    }

    #[test]
    fn an_unreadable_record_is_spent_and_never_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run");
        std::fs::write(&path, b"abc").unwrap();
        let s = FileRunStore::new(&path);
        assert!(s.last().is_err());
        assert_eq!(s.next().unwrap(), None);
        assert_eq!(std::fs::read(&path).unwrap(), b"abc", "never reset");
    }
}
