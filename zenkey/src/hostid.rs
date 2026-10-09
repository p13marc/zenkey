//! `hostid.v1`'s runtime: a system minted from the machine id
//! (`spec/profiles/hostid/v1.md` §2.3–§2.8, §2.13; #719, PB).
//!
//! The derivation itself (§2.1) is `zenkey-model`'s
//! ([`zenkey_model::hostid`]). This module reads the host:
//!
//! ```text
//! /etc/machine-id ─► /var/lib/dbus/machine-id ─► /var/lib/zk2/hostid
//!   absent or refused: the next      │           absent: created (§2.5),
//!   unreadable: fail closed (§2.6)   │           link(2), never rename
//!                                    ▼
//!            no id ─► fail closed, naming every path (§2.6),
//!                     or, with `hostid.ephemeral`, an ephemeral system
//!                     when the shared file was not created
//! ```
//!
//! - [`HostIdSource`] is one read of the inputs under a root (`/` on a
//!   host, a directory in a test), with nothing cached: scenarios §2's
//!   racer.
//! - [`HostIdMinter`] mints at most once per run (§2.7): the process's
//!   minted system, set by the first service that asks. A service's runtime
//!   uses [`HostIdMinter::global`]; a test hands its own to
//!   [`crate::ServiceBuilder::with_hostid`].
//! - [`HostIdError`] names every path tried, each with its outcome, and the
//!   operating system's error where there is one (§2.6).
//!
//! The machine id never leaves this module in any form but the system
//! derived from it (§2.10): no `Debug`, `Display` or accessor prints it.
//!
//! The paths named in errors and logs are always the absolute ones of
//! §2.4, whatever the root. A root other than `/` is a seam, and resolves
//! symbolic links as a chroot does: an absolute target starts at the root,
//! never at the host's `/` (text 0.2, F-94).

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, DirBuilder, File, OpenOptions, Permissions};
use std::io::{self, Read as _, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use rand::TryRngCore;
use rand::rngs::OsRng;
use zenkey_model::grammar::{Addr, Name};
use zenkey_model::hostid as derivation;

use crate::config::{Address, ServiceConfig};

/// The first input (§2.4).
pub const ETC_MACHINE_ID: &str = "/etc/machine-id";
/// The second input (§2.4).
pub const DBUS_MACHINE_ID: &str = "/var/lib/dbus/machine-id";
/// The shared file, the third input, created when it is absent (§2.5).
pub const SHARED_FILE: &str = "/var/lib/zk2/hostid";
/// The shared file's directory (§2.5 step 1).
pub const SHARED_DIR: &str = "/var/lib/zk2";
/// The inputs, in their order (§2.4). The order is part of the wire.
pub const INPUTS: [&str; 3] = [ETC_MACHINE_ID, DBUS_MACHINE_ID, SHARED_FILE];
/// A runtime reads at most this many bytes of an input; a larger regular
/// file is refused content (§2.4).
pub const READ_BOUND: u64 = 4096;

/// What one input came to (§2.4–§2.6).
#[derive(Debug)]
pub enum Outcome {
    /// It yielded the id the system is derived from.
    Id,
    /// The shared file was absent, and this run created it (§2.5): its
    /// content is the id.
    Created,
    /// Opening it failed with `ENOENT`, a dangling symbolic link included.
    /// For a machine-id file, the next input is tried.
    Absent,
    /// §2.1 refuses its content (empty, `uninitialized`, not 32 hex digits
    /// once normalised, all zeros, not UTF-8), or it is a regular file over
    /// the read bound. For a machine-id file, the next input is tried; the
    /// shared file is never replaced (§2.5).
    Refused,
    /// Any other failure to read it to its end, or not a regular file once
    /// symbolic links are followed: the runtime fails closed (§2.4).
    Unreadable(io::Error),
    /// The shared file was not created: a failure in §2.5's steps 1 to 4.
    NotCreated(io::Error),
}

impl Outcome {
    /// Whether the input yielded the id.
    #[must_use]
    pub fn is_id(&self) -> bool {
        matches!(self, Self::Id | Self::Created)
    }

    /// The outcome's name, as errors, logs and tools spell it: `id`,
    /// `created`, `absent`, `refused`, `unreadable`, `not created`.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Created => "created",
            Self::Absent => "absent",
            Self::Refused => "refused",
            Self::Unreadable(_) => "unreadable",
            Self::NotCreated(_) => "not created",
        }
    }

    /// The error behind an unreadable input or a file not created.
    #[must_use]
    pub fn error(&self) -> Option<&io::Error> {
        match self {
            Self::Unreadable(e) | Self::NotCreated(e) => Some(e),
            _ => None,
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused => f.write_str("refused by hostid.v1 §2.1"),
            Self::Unreadable(e) | Self::NotCreated(e) => write!(f, "{} ({e})", self.label()),
            _ => f.write_str(self.label()),
        }
    }
}

/// One input tried, by its absolute path (§2.4), and what it came to.
#[derive(Debug)]
pub struct Attempt {
    /// One of [`INPUTS`].
    pub path: &'static str,
    pub outcome: Outcome,
}

/// A trail of attempts, displayed as `path: outcome; …`.
#[derive(Debug, Clone, Copy)]
pub struct Trail<'a>(pub &'a [Attempt]);

impl fmt::Display for Trail<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, a) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{}: {}", a.path, a.outcome)?;
        }
        Ok(())
    }
}

/// Why `hostid.v1` gave a service no system.
#[derive(Debug)]
pub enum HostIdError {
    /// No id (§2.6): the service does not start, and declares nothing.
    NoId {
        /// The directory standing in for `/`; `/` on a host.
        root: PathBuf,
        /// Every path tried, in order, each with its outcome.
        trail: Vec<Attempt>,
        /// Whether `hostid.ephemeral` was set: it replaces only the refusal
        /// where every input gave no id and the shared file was not
        /// created.
        ephemeral: bool,
        /// The random source failed drawing an ephemeral id.
        draw: Option<io::Error>,
    },
    /// A configuration error (§2.3): the process's setting is fixed by the
    /// first service that asked for a minted system, and this one sets
    /// `hostid.ephemeral` otherwise.
    Setting { fixed: bool, asked: bool },
}

impl HostIdError {
    /// Every path tried, each with its outcome; empty for a configuration
    /// error, which reads no input.
    #[must_use]
    pub fn trail(&self) -> &[Attempt] {
        match self {
            Self::NoId { trail, .. } => trail,
            Self::Setting { .. } => &[],
        }
    }
}

impl fmt::Display for HostIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoId {
                root,
                trail,
                ephemeral,
                draw,
            } => {
                f.write_str("hostid.v1 found no id for this host")?;
                if root != Path::new("/") {
                    write!(f, " (under {})", root.display())?;
                }
                write!(
                    f,
                    ", so the service does not start (hostid.v1 §2.6): {}",
                    Trail(trail)
                )?;
                let not_created = trail
                    .last()
                    .is_some_and(|a| matches!(a.outcome, Outcome::NotCreated(_)));
                if let Some(e) = draw {
                    write!(f, "; drawing an ephemeral id failed ({e})")?;
                } else if not_created && !ephemeral {
                    f.write_str(
                        "; `hostid = { ephemeral = true }` would allow an ephemeral system",
                    )?;
                }
                Ok(())
            }
            Self::Setting { fixed, asked } => write!(
                f,
                "this service sets hostid.ephemeral = {asked}, but this process's first service \
                 that asked for a minted system set {fixed}: one process, one setting \
                 (hostid.v1 §2.3)"
            ),
        }
    }
}

impl std::error::Error for HostIdError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoId { draw: Some(e), .. } => Some(e),
            _ => None,
        }
    }
}

/// A system minted by `hostid.v1`, with how it was minted.
#[derive(Clone)]
pub struct Minted {
    system: Name,
    ephemeral: bool,
    trail: Arc<[Attempt]>,
    /// The normalised id (§2.1). Never printed (§2.10): only
    /// [`Minted::with_salt`] reads it.
    id: Arc<str>,
}

impl Minted {
    fn new(id: String, ephemeral: bool, trail: Vec<Attempt>) -> Self {
        let system = derivation::system(&id).expect("the id was normalised (§2.1)");
        Self {
            system,
            ephemeral,
            trail: trail.into(),
            id: id.into(),
        }
    }

    /// The system: `h-` and 12 hex digits (§2.1).
    #[must_use]
    pub fn system(&self) -> &Name {
        &self.system
    }

    /// Whether it is ephemeral (§2.6): drawn at random, written nowhere,
    /// living one run.
    #[must_use]
    pub fn is_ephemeral(&self) -> bool {
        self.ephemeral
    }

    /// Every path tried, in order, each with its outcome.
    #[must_use]
    pub fn trail(&self) -> &[Attempt] {
        &self.trail
    }

    /// The input the id came from; `None` for an ephemeral system.
    #[must_use]
    pub fn source(&self) -> Option<&'static str> {
        self.trail
            .iter()
            .find(|a| a.outcome.is_id())
            .map(|a| a.path)
    }

    /// What the same id derives with another salt (§2.1): a v1 origin,
    /// for the migration table (Appendix B). `None` for an ephemeral
    /// system, whose id no v1 run ever read.
    #[must_use]
    pub fn with_salt(&self, salt: &str) -> Option<String> {
        if self.ephemeral {
            return None;
        }
        derivation::derive(&self.id, salt)
    }
}

impl fmt::Debug for Minted {
    /// Without the id (§2.10).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Minted")
            .field("system", &self.system)
            .field("ephemeral", &self.ephemeral)
            .field("trail", &self.trail)
            .finish_non_exhaustive()
    }
}

/// A file operation of the ladder, for [`Fault`].
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Opening an input, one of [`INPUTS`] (§2.4).
    Open(&'static str),
    /// Creating the temporary file (§2.5 step 2).
    CreateTemp,
    /// `link(2)` of the temporary file to the shared file (§2.5 step 4).
    Link,
}

/// A failure a test injects where it cannot cause one: as root, modes do
/// not refuse, and no common file system refuses `link(2)` (scenarios'
/// conventions, "through the runtime's seam"). The step fails with the
/// operating system's error `errno`, as the system call would.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fault {
    pub step: Step,
    pub errno: i32,
}

/// Where `hostid.v1` reads a host's inputs (§2.4), and one read of them.
#[derive(Debug, Clone)]
pub struct HostIdSource {
    root: PathBuf,
    create: bool,
    faults: Vec<Fault>,
}

/// One component of a path being resolved under a root.
enum Part {
    Name(OsString),
    Up,
}

impl Part {
    fn of(c: Component<'_>) -> Option<Self> {
        match c {
            Component::Normal(n) => Some(Self::Name(n.to_owned())),
            Component::ParentDir => Some(Self::Up),
            Component::RootDir | Component::CurDir | Component::Prefix(_) => None,
        }
    }
}

/// What reading one input gave.
enum Input {
    Id(String),
    NoId(Outcome),
    Unreadable(io::Error),
}

impl HostIdSource {
    /// The host's own inputs, under `/`.
    #[must_use]
    pub fn host() -> Self {
        Self::at("/")
    }

    /// The inputs under `root`, a directory standing in for `/`: a test's,
    /// or a tool's.
    #[must_use]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            create: true,
            faults: Vec::new(),
        }
    }

    /// Never creates the shared file: an absent one is reported absent. For
    /// a tool that reads what a service would mint, and writes nothing.
    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.create = false;
        self
    }

    /// Injects a failure (tests only).
    #[doc(hidden)]
    #[must_use]
    pub fn with_fault(mut self, step: Step, errno: i32) -> Self {
        self.faults.push(Fault { step, errno });
        self
    }

    /// The directory standing in for `/`.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where an absolute path of §2.4 is under the root, its symbolic links
    /// resolved as a chroot resolves them (scenarios.md, "A root"; text
    /// 0.2, F-94): an absolute target starts again at the root, and `..`
    /// never climbs above it. A component that does not exist ends the
    /// walk, and the rest is appended as written, so that opening the
    /// result fails with `ENOENT`, as the system's own open would. Under
    /// `/` the system resolves the path itself.
    ///
    /// Errors: `ELOOP` after 40 links, `ENOTDIR` where a file stands for a
    /// directory, and whatever reading a link or a directory returns.
    pub fn path_of(&self, absolute: &str) -> io::Result<PathBuf> {
        if self.root == Path::new("/") {
            return Ok(PathBuf::from(absolute));
        }
        let mut todo: VecDeque<Part> = Path::new(absolute)
            .components()
            .filter_map(Part::of)
            .collect();
        let mut done = self.root.clone();
        let depth = done.components().count();
        let mut links = 0;
        while let Some(part) = todo.pop_front() {
            let name = match part {
                Part::Up => {
                    if done.components().count() > depth {
                        done.pop();
                    }
                    continue;
                }
                Part::Name(n) => n,
            };
            let here = done.join(&name);
            match fs::symlink_metadata(&here) {
                Ok(m) if m.file_type().is_symlink() => {
                    links += 1;
                    if links > 40 {
                        return Err(io::Error::from_raw_os_error(libc::ELOOP));
                    }
                    let target = fs::read_link(&here)?;
                    if target.is_absolute() {
                        done = self.root.clone();
                    }
                    for p in target.components().filter_map(Part::of).rev() {
                        todo.push_front(p);
                    }
                }
                Ok(m) if !m.is_dir() && !todo.is_empty() => {
                    return Err(io::Error::from_raw_os_error(libc::ENOTDIR));
                }
                Ok(_) => done.push(name),
                Err(e) if e.raw_os_error() == Some(libc::ENOENT) => {
                    done.push(name);
                    for p in todo.drain(..) {
                        if let Part::Name(n) = p {
                            done.push(n);
                        }
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(done)
    }

    fn fault(&self, step: Step) -> Option<io::Error> {
        self.faults
            .iter()
            .find(|f| f.step == step)
            .map(|f| io::Error::from_raw_os_error(f.errno))
    }

    /// Reads the inputs once, in order, and mints a system from the first
    /// that yields an id (§2.4–§2.6), creating the shared file when it is
    /// absent (§2.5). Nothing is cached: this is one racer of scenarios §2.
    /// A service's runtime asks a [`HostIdMinter`] instead, which mints
    /// once per run (§2.7).
    ///
    /// With `ephemeral`, a run where every input gave no id and the shared
    /// file was not created mints an ephemeral system (§2.6). An unreadable
    /// input, or a shared file without an id, fails closed whatever it says.
    pub fn mint(&self, ephemeral: bool) -> Result<Minted, HostIdError> {
        let mut trail = Vec::new();
        let fail = |trail, draw| HostIdError::NoId {
            root: self.root.clone(),
            trail,
            ephemeral,
            draw,
        };
        for path in [ETC_MACHINE_ID, DBUS_MACHINE_ID] {
            match self.read(path) {
                Input::Id(id) => {
                    trail.push(Attempt {
                        path,
                        outcome: Outcome::Id,
                    });
                    return Ok(Minted::new(id, false, trail));
                }
                Input::NoId(outcome) => trail.push(Attempt { path, outcome }),
                Input::Unreadable(e) => {
                    trail.push(Attempt {
                        path,
                        outcome: Outcome::Unreadable(e),
                    });
                    return Err(fail(trail, None));
                }
            }
        }
        let outcome = match self.read(SHARED_FILE) {
            Input::Id(id) => {
                trail.push(Attempt {
                    path: SHARED_FILE,
                    outcome: Outcome::Id,
                });
                return Ok(Minted::new(id, false, trail));
            }
            Input::NoId(Outcome::Absent) if self.create => match self.create_shared() {
                Ok(id) => {
                    trail.push(Attempt {
                        path: SHARED_FILE,
                        outcome: Outcome::Created,
                    });
                    return Ok(Minted::new(id, false, trail));
                }
                // §2.5 step 4: another racer won. Its file is an input like
                // any other: an id, or a failure.
                Err(Creation::Lost(Input::Id(id))) => {
                    trail.push(Attempt {
                        path: SHARED_FILE,
                        outcome: Outcome::Id,
                    });
                    return Ok(Minted::new(id, false, trail));
                }
                Err(Creation::Lost(Input::NoId(o))) => o,
                Err(Creation::Lost(Input::Unreadable(e))) => Outcome::Unreadable(e),
                Err(Creation::Failed(e)) => Outcome::NotCreated(e),
            },
            Input::NoId(o) => o,
            Input::Unreadable(e) => Outcome::Unreadable(e),
        };
        let not_created = matches!(outcome, Outcome::NotCreated(_));
        trail.push(Attempt {
            path: SHARED_FILE,
            outcome,
        });
        if !(ephemeral && not_created) {
            return Err(fail(trail, None));
        }
        // §2.6: drawn as §2.5 draws, derived by §2.1, written nowhere.
        match random_id() {
            Ok(id) => Ok(Minted::new(id, true, trail)),
            Err(e) => Err(fail(trail, Some(e))),
        }
    }

    /// Reads one input (§2.4): its bytes, at most [`READ_BOUND`] of them,
    /// symbolic links followed.
    fn read(&self, path: &'static str) -> Input {
        let opened = match self.fault(Step::Open(path)) {
            Some(e) => Err(e),
            // O_NONBLOCK: a FIFO opens at once instead of waiting for a
            // writer, and is then refused as not a regular file. It changes
            // nothing for a regular file.
            None => self.path_of(path).and_then(|p| {
                OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(p)
            }),
        };
        let file = match opened {
            Ok(f) => f,
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => {
                return Input::NoId(Outcome::Absent);
            }
            Err(e) => return Input::Unreadable(e),
        };
        let meta = match file.metadata() {
            Ok(m) => m,
            Err(e) => return Input::Unreadable(e),
        };
        let kind = meta.file_type();
        if !kind.is_file() {
            let what = if kind.is_dir() {
                "a directory"
            } else if kind.is_fifo() {
                "a FIFO"
            } else if kind.is_socket() {
                "a socket"
            } else if kind.is_char_device() || kind.is_block_device() {
                "a device"
            } else {
                "not a regular file"
            };
            return Input::Unreadable(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("not a regular file: {what}"),
            ));
        }
        if meta.len() > READ_BOUND {
            return Input::NoId(Outcome::Refused);
        }
        let mut bytes = Vec::with_capacity(64);
        if let Err(e) = file.take(READ_BOUND).read_to_end(&mut bytes) {
            return Input::Unreadable(e);
        }
        // Bytes that are not UTF-8 are not 32 hex digits (§2.4).
        match std::str::from_utf8(&bytes)
            .ok()
            .and_then(derivation::normalise)
        {
            Some(id) => Input::Id(id),
            None => Input::NoId(Outcome::Refused),
        }
    }

    /// Creates the shared file (§2.5): a temporary file, written, synced,
    /// then `link(2)`ed to its final name, so that every racer ends with
    /// the same complete content. Never `rename(2)`, never in place.
    fn create_shared(&self) -> Result<String, Creation> {
        // 1. The directory, and any missing parent; mode 0755.
        let dir = self.path_of(SHARED_DIR).map_err(Creation::Failed)?;
        make_dirs(&dir).map_err(Creation::Failed)?;
        let id = random_id().map_err(Creation::Failed)?;
        let tmp = dir.join(format!(
            ".hostid.{}",
            random_hex(8).map_err(Creation::Failed)?
        ));
        // 2. Exclusively, under a name no racer shares, mode 0644 whatever
        // the umask.
        let created = match self.fault(Step::CreateTemp) {
            Some(e) => Err(e),
            None => OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o644)
                .open(&tmp),
        };
        let mut file = created.map_err(Creation::Failed)?;
        let published = (|| {
            file.set_permissions(Permissions::from_mode(0o644))?;
            // 3. The content, then fsync(2).
            file.write_all(format!("{id}\n").as_bytes())?;
            file.sync_all()?;
            drop(file);
            // 4. link(2): only a complete file is ever under the final name.
            match self.fault(Step::Link) {
                Some(e) => Err(e),
                None => fs::hard_link(&tmp, dir.join("hostid")),
            }
        })();
        // 5. The temporary file goes, whatever happened after step 2.
        if let Err(e) = fs::remove_file(&tmp)
            && e.raw_os_error() != Some(libc::ENOENT)
        {
            tracing::warn!(
                path = %tmp.display(),
                error = %e,
                "hostid.v1: the temporary file could not be removed (§2.5 step 5)"
            );
        }
        match published {
            Ok(()) => {
                if let Err(e) = File::open(&dir).and_then(|d| d.sync_all()) {
                    tracing::warn!(
                        path = SHARED_DIR,
                        error = %e,
                        "hostid.v1: the directory's fsync failed; the shared file stands (§2.5 step 4)"
                    );
                }
                Ok(id)
            }
            Err(e) if e.raw_os_error() == Some(libc::EEXIST) => {
                Err(Creation::Lost(self.read(SHARED_FILE)))
            }
            Err(e) => Err(Creation::Failed(e)),
        }
    }
}

/// Why this run did not create the shared file.
enum Creation {
    /// Another racer's file was there first: this is what reading it gave.
    Lost(Input),
    /// A failure in steps 1 to 4: not created.
    Failed(io::Error),
}

/// Creates `dir` and its missing parents, each new one with mode 0755
/// whatever the umask. One that already exists, made by a racer or not,
/// is success (§2.5 step 1).
fn make_dirs(dir: &Path) -> io::Result<()> {
    if fs::metadata(dir).is_ok_and(|m| m.is_dir()) {
        return Ok(());
    }
    if let Some(parent) = dir.parent() {
        make_dirs(parent)?;
    }
    match DirBuilder::new().mode(0o755).create(dir) {
        Ok(()) => fs::set_permissions(dir, Permissions::from_mode(0o755)),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

/// `n` bytes from the operating system's CSPRNG, as lowercase hex.
fn random_hex(n: usize) -> io::Result<String> {
    let mut bytes = vec![0u8; n];
    OsRng.try_fill_bytes(&mut bytes).map_err(|e| {
        e.raw_os_error().map_or_else(
            || io::Error::other("the random source failed"),
            io::Error::from_raw_os_error,
        )
    })?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// A fresh id: the hex of 16 bytes from the CSPRNG (§2.5). The one draw
/// §2.1 would refuse, all zeros (a chance of 2⁻¹²⁸), is drawn again.
fn random_id() -> io::Result<String> {
    loop {
        let id = random_hex(16)?;
        if derivation::normalise(&id).is_some() {
            return Ok(id);
        }
    }
}

/// Mints a process's system at most once (§2.7).
///
/// The first service that asks fixes the process's `hostid.ephemeral`
/// setting (§2.3), and the first that gets a system fixes the system:
/// every later one gets the same, whatever the inputs say by then. A failed
/// ask mints nothing, so a later ask with the same setting reads the inputs
/// again.
#[derive(Debug)]
pub struct HostIdMinter {
    source: HostIdSource,
    state: Mutex<MintState>,
}

#[derive(Debug, Default)]
struct MintState {
    setting: Option<bool>,
    minted: Option<Minted>,
}

impl HostIdMinter {
    /// A minter reading `source`: a test's, on a root of its own.
    #[must_use]
    pub fn new(source: HostIdSource) -> Self {
        Self {
            source,
            state: Mutex::default(),
        }
    }

    /// The process's minter, reading the host's own inputs: the one
    /// [`crate::ServiceBuilder::new`] uses.
    pub fn global() -> &'static Self {
        static GLOBAL: OnceLock<HostIdMinter> = OnceLock::new();
        GLOBAL.get_or_init(|| Self::new(HostIdSource::host()))
    }

    /// Where it reads.
    #[must_use]
    pub fn source(&self) -> &HostIdSource {
        &self.source
    }

    /// The process's system, minted on the first ask (§2.7). A later ask
    /// whose `ephemeral` differs from the first's is a configuration error
    /// (§2.3).
    pub fn system(&self, ephemeral: bool) -> Result<Minted, HostIdError> {
        let mut state = self.state.lock().expect("not poisoned");
        match state.setting {
            Some(fixed) if fixed != ephemeral => {
                return Err(HostIdError::Setting {
                    fixed,
                    asked: ephemeral,
                });
            }
            Some(_) => {}
            None => state.setting = Some(ephemeral),
        }
        if let Some(m) = &state.minted {
            return Ok(m.clone());
        }
        // Minted under the lock: two services starting at once in one
        // process mint once.
        let m = self.source.mint(ephemeral)?;
        state.minted = Some(m.clone());
        Ok(m)
    }

    /// The system this process minted, if it has.
    #[must_use]
    pub fn minted(&self) -> Option<Minted> {
        self.state.lock().expect("not poisoned").minted.clone()
    }
}

/// A configuration's address, resolved (§2.7).
#[derive(Debug, Clone)]
pub struct Resolved {
    pub address: Addr,
    /// How the system was minted; `None` for a literal one.
    pub minted: Option<Minted>,
}

impl ServiceConfig {
    /// Resolves the address with the process's minter
    /// ([`HostIdMinter::global`]). A runtime calls it before its session
    /// opens where it can (§2.7); a literal address reads nothing.
    pub fn resolve(&self) -> Result<Resolved, HostIdError> {
        self.resolve_with(HostIdMinter::global())
    }

    /// Resolves the address with `minter`.
    pub fn resolve_with(&self, minter: &HostIdMinter) -> Result<Resolved, HostIdError> {
        match &self.address {
            Address::Literal(address) => Ok(Resolved {
                address: address.clone(),
                minted: None,
            }),
            Address::HostId { service, ephemeral } => {
                let m = minter.system(*ephemeral)?;
                Ok(Resolved {
                    address: Addr {
                        system: m.system().clone(),
                        service: service.clone(),
                    },
                    minted: Some(m),
                })
            }
        }
    }
}

/// This host's name, for `meta.host` (§2.13): informative, never an
/// identity. `None` when the system will not say.
#[must_use]
pub fn hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: `gethostname` writes at most `buf.len()` bytes into `buf`.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let name = std::str::from_utf8(&buf[..end]).ok()?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const M1: &str = "b642b4217b34b1e8d3bd915fc65c4452";
    const S1: &str = "h-bbd1aa1db10b";
    const M2: &str = "0123456789abcdef0123456789abcdef";
    const S2: &str = "h-3f6d94515669";

    fn root() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("var/lib")).unwrap();
        d
    }

    fn put(root: &Path, abs: &str, content: &[u8]) {
        let p = root.join(abs.trim_start_matches('/'));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    fn outcomes(trail: &[Attempt]) -> Vec<(&'static str, &'static str)> {
        trail.iter().map(|a| (a.path, a.outcome.label())).collect()
    }

    fn is_root() -> bool {
        // SAFETY: `geteuid` cannot fail.
        unsafe { libc::geteuid() == 0 }
    }

    #[test]
    fn the_first_input_that_yields_an_id_wins() {
        let r = root();
        put(r.path(), ETC_MACHINE_ID, format!("{M1}\n").as_bytes());
        put(r.path(), DBUS_MACHINE_ID, format!("{M2}\n").as_bytes());
        let m = HostIdSource::at(r.path()).mint(false).unwrap();
        assert_eq!(m.system().as_str(), S1);
        assert_eq!(m.source(), Some(ETC_MACHINE_ID));
        assert!(!r.path().join("var/lib/zk2").exists());

        put(r.path(), ETC_MACHINE_ID, b"uninitialized\n");
        let m = HostIdSource::at(r.path()).mint(false).unwrap();
        assert_eq!(m.system().as_str(), S2);
        assert_eq!(
            outcomes(m.trail()),
            [(ETC_MACHINE_ID, "refused"), (DBUS_MACHINE_ID, "id")]
        );
    }

    /// §2.4: a dangling link is absent; a link to an id is followed.
    #[test]
    fn symbolic_links_are_followed() {
        let r = root();
        put(r.path(), ETC_MACHINE_ID, format!("{M2}\n").as_bytes());
        fs::create_dir_all(r.path().join("var/lib/dbus")).unwrap();
        std::os::unix::fs::symlink(
            "../../../etc/machine-id",
            r.path().join("var/lib/dbus/machine-id"),
        )
        .unwrap();
        fs::remove_file(r.path().join("etc/machine-id")).unwrap();
        // Dangling: absent, and the shared file is created.
        let m = HostIdSource::at(r.path()).mint(false).unwrap();
        assert_eq!(
            outcomes(m.trail()),
            [
                (ETC_MACHINE_ID, "absent"),
                (DBUS_MACHINE_ID, "absent"),
                (SHARED_FILE, "created")
            ]
        );
        // Restored, the link is followed: the second input's id.
        let r = root();
        put(r.path(), ETC_MACHINE_ID, b"");
        fs::create_dir_all(r.path().join("var/lib/dbus")).unwrap();
        put(r.path(), "/elsewhere", format!("{M2}\n").as_bytes());
        std::os::unix::fs::symlink(
            "../../../elsewhere",
            r.path().join("var/lib/dbus/machine-id"),
        )
        .unwrap();
        let m = HostIdSource::at(r.path()).mint(false).unwrap();
        assert_eq!(m.system().as_str(), S2);
    }

    /// Text 0.2 (F-94): under a root, an absolute link starts at the root,
    /// as under a chroot, never at the host's `/`.
    #[test]
    fn a_root_resolves_links_as_a_chroot() {
        let r = root();
        fs::create_dir_all(r.path().join("var/lib/dbus")).unwrap();
        std::os::unix::fs::symlink("/etc/machine-id", r.path().join("var/lib/dbus/machine-id"))
            .unwrap();
        // Dangling under the root, whatever the host's own file holds.
        let m = HostIdSource::at(r.path())
            .read_only()
            .mint(false)
            .unwrap_err();
        assert_eq!(
            outcomes(m.trail()),
            [
                (ETC_MACHINE_ID, "absent"),
                (DBUS_MACHINE_ID, "absent"),
                (SHARED_FILE, "absent")
            ]
        );
        // An absolute link, and `..` past the root, both land in it.
        put(r.path(), ETC_MACHINE_ID, b"uninitialized\n");
        put(r.path(), "/srv/id", format!("{M2}\n").as_bytes());
        fs::remove_file(r.path().join("var/lib/dbus/machine-id")).unwrap();
        std::os::unix::fs::symlink(
            "../../../../../../srv/id",
            r.path().join("var/lib/dbus/machine-id"),
        )
        .unwrap();
        assert_eq!(
            HostIdSource::at(r.path())
                .mint(false)
                .unwrap()
                .system()
                .as_str(),
            S2
        );
        fs::remove_file(r.path().join("var/lib/dbus/machine-id")).unwrap();
        std::os::unix::fs::symlink("/srv/id", r.path().join("var/lib/dbus/machine-id")).unwrap();
        assert_eq!(
            HostIdSource::at(r.path())
                .mint(false)
                .unwrap()
                .system()
                .as_str(),
            S2
        );
    }

    /// §2.4: the read bound, and bytes that are not UTF-8.
    #[test]
    fn content_is_bytes_within_a_bound() {
        let r = root();
        let mut padded = format!("{M1}\n").into_bytes();
        padded.resize(4096, b'\n');
        put(r.path(), ETC_MACHINE_ID, &padded);
        assert_eq!(
            HostIdSource::at(r.path())
                .mint(false)
                .unwrap()
                .system()
                .as_str(),
            S1,
            "4,096 bytes are read"
        );
        padded.push(b'\n');
        put(r.path(), ETC_MACHINE_ID, &padded);
        put(r.path(), DBUS_MACHINE_ID, b"\xff\xfe");
        put(r.path(), SHARED_FILE, format!("{M2}\n").as_bytes());
        let m = HostIdSource::at(r.path()).mint(false).unwrap();
        assert_eq!(
            outcomes(m.trail()),
            [
                (ETC_MACHINE_ID, "refused"),
                (DBUS_MACHINE_ID, "refused"),
                (SHARED_FILE, "id")
            ]
        );
    }

    /// §2.4: anything but an absent input or refused content fails closed:
    /// a directory, a FIFO, a link loop, a parent that is a file.
    #[test]
    fn what_is_not_read_fails_closed() {
        let cases: [(&str, fn(&Path)); 4] = [
            ("a directory", |r| {
                fs::create_dir_all(r.join("etc/machine-id")).unwrap();
            }),
            ("a FIFO", |r| {
                fs::create_dir_all(r.join("etc")).unwrap();
                let p = std::ffi::CString::new(r.join("etc/machine-id").to_str().unwrap()).unwrap();
                // SAFETY: a valid C string and a mode.
                assert_eq!(unsafe { libc::mkfifo(p.as_ptr(), 0o644) }, 0);
            }),
            ("a loop", |r| {
                fs::create_dir_all(r.join("etc")).unwrap();
                std::os::unix::fs::symlink("machine-id", r.join("etc/machine-id")).unwrap();
            }),
            ("a parent that is a file", |r| {
                fs::write(r.join("etc"), b"").unwrap();
            }),
        ];
        for (what, make) in cases {
            let r = root();
            make(r.path());
            put(r.path(), DBUS_MACHINE_ID, format!("{M2}\n").as_bytes());
            let e = HostIdSource::at(r.path()).mint(true).unwrap_err();
            assert_eq!(
                outcomes(e.trail()),
                [(ETC_MACHINE_ID, "unreadable")],
                "{what}: {e}"
            );
            assert!(e.trail()[0].outcome.error().is_some(), "{what}");
            assert!(!r.path().join("var/lib/zk2").exists(), "{what}");
        }
    }

    /// §2.4: unreadable fails closed with the operating system's error, by
    /// a real mode where the test can be refused one, by the seam as well.
    #[test]
    fn an_unreadable_machine_id_fails_closed() {
        let r = root();
        put(r.path(), ETC_MACHINE_ID, format!("{M1}\n").as_bytes());
        let mut sources =
            vec![HostIdSource::at(r.path()).with_fault(Step::Open(ETC_MACHINE_ID), libc::EACCES)];
        if !is_root() {
            fs::set_permissions(r.path().join("etc"), Permissions::from_mode(0o000)).unwrap();
            sources.push(HostIdSource::at(r.path()));
        }
        for s in sources {
            let e = s.mint(true).unwrap_err();
            let [a] = e.trail() else { panic!("{e}") };
            assert_eq!(a.path, ETC_MACHINE_ID);
            assert_eq!(
                a.outcome.error().and_then(io::Error::raw_os_error),
                Some(libc::EACCES)
            );
            assert!(e.to_string().contains("/etc/machine-id: unreadable"), "{e}");
        }
        fs::set_permissions(r.path().join("etc"), Permissions::from_mode(0o755)).unwrap();
        assert!(!r.path().join("var/lib/zk2").exists());
    }

    /// §2.5: created with its mode and content; then read, never replaced.
    #[test]
    fn the_shared_file_is_created_once_and_never_replaced() {
        let r = root();
        let first = HostIdSource::at(r.path()).mint(false).unwrap();
        let path = r.path().join("var/lib/zk2/hostid");
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content.len(), 33);
        assert!(content.ends_with('\n'));
        assert!(derivation::normalise(&content).is_some());
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(
            fs::metadata(r.path().join("var/lib/zk2"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(
            fs::read_dir(r.path().join("var/lib/zk2")).unwrap().count(),
            1
        );
        let again = HostIdSource::at(r.path()).mint(false).unwrap();
        assert_eq!(again.system(), first.system());
        assert_eq!(outcomes(again.trail())[2], (SHARED_FILE, "id"));

        // An existing file without an id stays as it is (§2.5).
        put(r.path(), SHARED_FILE, b"garbage\n");
        let e = HostIdSource::at(r.path()).mint(true).unwrap_err();
        assert_eq!(outcomes(e.trail())[2], (SHARED_FILE, "refused"));
        assert_eq!(fs::read(&path).unwrap(), b"garbage\n");
    }

    /// §2.5 step 4: `link(2)` refused leaves no file of either name;
    /// `read_only` creates nothing.
    #[test]
    fn a_shared_file_not_created() {
        let r = root();
        let e = HostIdSource::at(r.path())
            .with_fault(Step::Link, libc::EPERM)
            .mint(false)
            .unwrap_err();
        assert_eq!(
            outcomes(e.trail()),
            [
                (ETC_MACHINE_ID, "absent"),
                (DBUS_MACHINE_ID, "absent"),
                (SHARED_FILE, "not created")
            ]
        );
        assert_eq!(
            e.trail()[2]
                .outcome
                .error()
                .and_then(io::Error::raw_os_error),
            Some(libc::EPERM)
        );
        assert!(e.to_string().contains("ephemeral = true"), "{e}");
        assert_eq!(
            fs::read_dir(r.path().join("var/lib/zk2")).unwrap().count(),
            0
        );

        let e = HostIdSource::at(r.path())
            .read_only()
            .mint(false)
            .unwrap_err();
        assert_eq!(outcomes(e.trail())[2], (SHARED_FILE, "absent"));
        assert!(!r.path().join("var/lib/zk2/hostid").exists());
    }

    /// Text 0.2 (F-96): `EEXIST`, then the winner's file absent when read,
    /// fails closed, ephemeral or not: the host had an id that another
    /// service may hold.
    #[test]
    fn a_winner_that_vanished_fails_closed() {
        let r = root();
        let e = HostIdSource::at(r.path())
            .with_fault(Step::Link, libc::EEXIST)
            .mint(true)
            .unwrap_err();
        assert_eq!(outcomes(e.trail())[2], (SHARED_FILE, "absent"));
        assert_eq!(
            fs::read_dir(r.path().join("var/lib/zk2")).unwrap().count(),
            0
        );
    }

    /// §2.6: ephemeral replaces exactly the refusal of a file not created.
    #[test]
    fn ephemeral_is_a_fallback() {
        let r = root();
        let s = HostIdSource::at(r.path()).with_fault(Step::CreateTemp, libc::EACCES);
        let a = s.mint(true).unwrap();
        let b = s.mint(true).unwrap();
        assert!(a.is_ephemeral());
        assert!(derivation::is_minted_shape(a.system().as_str()));
        assert_ne!(a.system(), b.system());
        assert_eq!(a.source(), None);
        assert_eq!(a.with_salt("x"), None);
        assert!(!format!("{a:?}").contains("id:"), "the id is never printed");
        put(r.path(), ETC_MACHINE_ID, format!("{M1}\n").as_bytes());
        assert_eq!(s.mint(true).unwrap().system().as_str(), S1);
    }

    /// §2.7: once per run; one setting per process.
    #[test]
    fn a_minter_mints_once() {
        let r = root();
        put(r.path(), ETC_MACHINE_ID, format!("{M1}\n").as_bytes());
        let m = HostIdMinter::new(HostIdSource::at(r.path()));
        assert!(m.minted().is_none());
        assert_eq!(m.system(false).unwrap().system().as_str(), S1);
        put(r.path(), ETC_MACHINE_ID, format!("{M2}\n").as_bytes());
        assert_eq!(m.system(false).unwrap().system().as_str(), S1);
        assert!(matches!(
            m.system(true),
            Err(HostIdError::Setting {
                fixed: false,
                asked: true
            })
        ));
        // Appendix B: the same id under v1's tcgui salt.
        assert_eq!(
            m.minted().unwrap().with_salt("tcgui-host-id-v1").as_deref(),
            Some("h-eed9ac91da20")
        );
    }

    #[test]
    fn this_host_has_a_name() {
        assert!(hostname().is_some_and(|h| !h.is_empty()));
    }
}
