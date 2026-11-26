//! A name the engine can open for a picked document.
//!
//! The engine opens tracks by path (and reads them again by path when it
//! seeks: the MP3 frame table, an Ogg file's links, an MP4's edit list). A
//! picked document has no path, only a content:// URI the app may open
//! through the system. So each track about to play is opened there (a file
//! descriptor) and given a name in the app's own cache:
//!
//! - a symbolic link to `/proc/self/fd/N` - the engine opens the very file
//!   the system handed over, nothing is copied, for as long as the
//!   descriptor is kept;
//! - where the link cannot be opened (a provider whose descriptors cannot be
//!   opened a second time: a pipe, some cloud storage) the document is copied
//!   into the cache instead - at most [`COPY_BYTES`], within [`COPY_TIME`].
//!
//! The opening itself may wait on the system for long: a provider that is
//! busy (a phone short of storage, its media scanner going through the
//! files being copied in) answers in seconds, or not at all. So it runs on
//! a thread of its own ([`Opener::begin`]) that the caller waits for no
//! longer than it likes ([`Opening`]); one given up on cleans up after
//! itself when it is done.
//!
//! A document still being written - copied in while it is picked - is not
//! played half: empty, changed in the last [`RECENT`], or growing between
//! two looks, it is "still being copied" ([`Failure::Copying`]) and the
//! player tries it again later.
//!
//! Only the last few tracks stay open ([`KEEP`]): the one playing, the one
//! queued behind it, the ones just left.

use crate::picked;
use crossbeam_channel::{Receiver, Sender};
use std::collections::VecDeque;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// How many opened tracks are kept (their descriptors, links or copies).
pub const KEEP: usize = 6;

/// A file changed less than this long ago is still being written.
pub const RECENT: Duration = Duration::from_secs(5);

/// A file changed less than this long ago is looked at twice, this far
/// apart: if it grew, it is still being written.
pub const SETTLING: Duration = Duration::from_secs(60);
pub const SECOND_LOOK: Duration = Duration::from_millis(300);

/// How long the system may take to hand over a document.
pub const OPEN_TIME: Duration = Duration::from_secs(12);

/// A copy (where a document cannot be opened in place) ends here; the
/// copies kept come to no more than [`COPY_TOTAL`] (the oldest go first -
/// never the one playing or the one queued behind it).
pub const COPY_TIME: Duration = Duration::from_secs(60);
pub const COPY_BYTES: u64 = 512 << 20;
pub const COPY_TOTAL: u64 = 512 << 20;

/// At most this many documents are being opened at once: a provider that
/// does not answer holds a thread each (until the Java side's cancel ends
/// its call), and no more than these pile up.
pub const OPENINGS: usize = 4;

/// One of the [`OPENINGS`], given back when its worker ends.
struct Slot(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Slot {
    fn take(count: &std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Option<Slot> {
        use std::sync::atomic::Ordering;
        let mut now = count.load(Ordering::Acquire);
        while now < OPENINGS {
            match count.compare_exchange_weak(now, now + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(Slot(count.clone())),
                Err(seen) => now = seen,
            }
        }
        None
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// Whether `uri` is a document of a content provider (else: a path).
pub fn is_document(uri: &str) -> bool {
    uri.starts_with("content://")
}

/// Why a document is not open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Still being written (copied in) - its size so far - or the system
    /// too busy to open it now: to be tried again later.
    Copying(String, u64),
    /// It cannot be opened or read.
    Failed(String),
}

impl Failure {
    pub fn message(&self) -> &str {
        match self {
            Failure::Copying(m, _) | Failure::Failed(m) => m,
        }
    }
}

/// What a worker says about the document it opens.
pub(crate) enum Said {
    /// The document cannot be opened in place: it is being copied (the
    /// caller gives it [`COPY_TIME`] more).
    Copying,
    Done(Result<Placed, Failure>),
}

pub struct Placed {
    path: PathBuf,
    /// The descriptor the link points at (None: a copy, or a plain path).
    file: Option<File>,
    /// The bytes of a copy (0: no copy).
    copied: u64,
    /// The path is this app's (a link or a copy in the cache), not the
    /// user's own file: it may be removed.
    own: bool,
}

/// A document being opened on a thread of its own.
pub struct Opening {
    pub uri: String,
    said: Receiver<Said>,
    /// When it is given up on (later once it copies).
    pub deadline: Instant,
    pub started: Instant,
    /// Told that it is given up on: what it opened is let go of.
    abandon: Option<Sender<()>>,
}

/// What an opening gave: the file's place in the cache and the descriptor
/// its link points at (None: a copy), or why not.
pub type Opened = Result<Placed, Failure>;

impl Opening {
    /// The worker's messages, for a `select!`: each one goes to
    /// [`Opening::handle`].
    pub(crate) fn said(&self) -> &Receiver<Said> {
        &self.said
    }

    /// A message of the worker's: Some when it is done.
    pub(crate) fn handle(
        &mut self,
        said: Result<Said, crossbeam_channel::RecvError>,
    ) -> Option<Opened> {
        match said {
            Ok(Said::Copying) => {
                self.deadline = Instant::now() + COPY_TIME + Duration::from_secs(2);
                None
            }
            Ok(Said::Done(result)) => Some(result),
            Err(_) => Some(Err(Failure::Failed(
                "the document could not be opened".into(),
            ))),
        }
    }

    /// What the worker said by now, without waiting: Some when it is done.
    pub fn take(&mut self) -> Option<Opened> {
        match self.said.try_recv() {
            Ok(said) => self.handle(Ok(said)),
            Err(crossbeam_channel::TryRecvError::Empty) => None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.handle(Err(crossbeam_channel::RecvError))
            }
        }
    }

    /// Waits for the worker at most `time`: Some when it is done.
    pub fn wait(&mut self, time: Duration) -> Option<Opened> {
        match self.said.recv_timeout(time) {
            Ok(said) => self.handle(Ok(said)),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => None,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                self.handle(Err(crossbeam_channel::RecvError))
            }
        }
    }

    /// No longer waited for: the worker lets go of what it opens.
    pub fn abandon(mut self) {
        self.abandon.take();
    }
}

impl Drop for Opening {
    fn drop(&mut self) {
        // (dropping the sender tells the worker no one waits any more)
        self.abandon.take();
    }
}

/// What [`Opener::begin`] gives.
pub enum Begun {
    /// The path at once (a plain path, or a document open already).
    Ready(String),
    /// Being opened.
    Opening(Opening),
    Failed(Failure),
}

struct Open {
    uri: String,
    path: PathBuf,
    /// The descriptor the link points at (None: a copy).
    _file: Option<File>,
    copied: u64,
}

pub struct Opener {
    /// The documents being opened now (see [`OPENINGS`]).
    openings: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    dir: Option<PathBuf>,
    open_fd: fn(&str) -> Option<i32>,
    open: VecDeque<Open>,
    count: u64,
}

impl Opener {
    pub fn new(dir: Option<PathBuf>, open_fd: fn(&str) -> Option<i32>) -> Opener {
        // (what an earlier run left: links to descriptors long closed)
        if let Some(dir) = &dir {
            let _ = std::fs::remove_dir_all(dir);
            let _ = std::fs::create_dir_all(dir);
        }
        Opener {
            openings: Default::default(),
            dir,
            open_fd,
            open: VecDeque::new(),
            count: 0,
        }
    }

    /// Starts opening `uri` (named `name`, whose ending is the decoder's
    /// hint; `size`: what the provider says it holds, where it says). The
    /// caller waits on the [`Opening`] and hands what it gives to
    /// [`Opener::finish`].
    pub fn begin(&mut self, uri: &str, name: &str, size: Option<u64>) -> Begun {
        if let Some(at) = self.open.iter().position(|o| o.uri == uri) {
            // (used again: the last to go)
            let again = self.open.remove(at).expect("the place just found");
            let path = path_text(&again.path);
            self.open.push_back(again);
            return Begun::Ready(path);
        }
        let document = is_document(uri);
        let path = if document {
            let Some(dir) = self.dir.clone() else {
                return Begun::Failed(Failure::Failed("no place to open documents in".into()));
            };
            self.count += 1;
            let ending = picked::extension(name).unwrap_or_else(|| "bin".to_owned());
            dir.join(format!("{}.{ending}", self.count))
        } else {
            PathBuf::from(uri)
        };
        let Some(slot) = Slot::take(&self.openings) else {
            return Begun::Failed(Failure::Copying(
                format!("the system is busy ({OPENINGS} documents still being opened)"),
                0,
            ));
        };
        let (tx, said) = crossbeam_channel::unbounded();
        let (abandon, abandoned) = crossbeam_channel::bounded::<()>(0);
        let open_fd = self.open_fd;
        let target = uri.to_owned();
        let started = Instant::now();
        let spawned = std::thread::Builder::new()
            .name("amp-open".into())
            .spawn(move || {
                let _slot = slot;
                let result = if document {
                    open_and_place(open_fd, &target, &path, size, &tx, &abandoned)
                } else {
                    check_path(&path, size)
                };
                // (no one waits any more: what was opened goes - a link or a
                // copy of ours, never the user's own file)
                if let Err(crossbeam_channel::SendError(Said::Done(Ok(placed)))) =
                    tx.send(Said::Done(result))
                {
                    drop(placed.file);
                    if placed.own {
                        let _ = std::fs::remove_file(&placed.path);
                    }
                }
            });
        if spawned.is_err() {
            return Begun::Failed(Failure::Failed("no thread to open the document on".into()));
        }
        Begun::Opening(Opening {
            uri: uri.to_owned(),
            said,
            deadline: started + OPEN_TIME,
            started,
            abandon: Some(abandon),
        })
    }

    /// What an [`Opening`] opened, kept: its path. Older ones go to keep
    /// [`KEEP`] and [`COPY_TOTAL`] - never one of `in_use` (the track
    /// playing, the one queued behind it: the engine reads them again by
    /// path when it seeks or the output changes).
    pub fn finish(&mut self, uri: &str, placed: Placed, in_use: &[&str]) -> String {
        let path = path_text(&placed.path);
        if !placed.own {
            // (a plain path: nothing of ours to keep or to remove)
            return path;
        }
        self.open.push_back(Open {
            uri: uri.to_owned(),
            path: placed.path,
            _file: placed.file,
            copied: placed.copied,
        });
        let pinned = |o: &Open| o.uri == uri || in_use.contains(&o.uri.as_str());
        loop {
            let copies: u64 = self.open.iter().map(|o| o.copied).sum();
            if self.open.len() <= KEEP && copies <= COPY_TOTAL {
                break;
            }
            let Some(at) = self.open.iter().position(|o| !pinned(o)) else {
                break;
            };
            if let Some(old) = self.open.remove(at) {
                let _ = std::fs::remove_file(&old.path);
            }
        }
        path
    }

    /// Opens `uri` and waits for it, at most until its deadline (the tests;
    /// the player's thread waits on [`Opener::begin`] itself).
    pub fn open(&mut self, uri: &str, name: &str) -> Result<String, Failure> {
        match self.begin(uri, name, None) {
            Begun::Ready(path) => Ok(path),
            Begun::Failed(failure) => Err(failure),
            Begun::Opening(mut opening) => loop {
                let left = opening.deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    opening.abandon();
                    return Err(Failure::Failed(too_slow()));
                }
                if let Some(result) = opening.wait(left) {
                    return Ok(self.finish(uri, result?, &[]));
                }
            },
        }
    }

    /// How many tracks are open now.
    pub fn kept(&self) -> usize {
        self.open.len()
    }
}

/// The message for a document the system did not hand over in time.
pub fn too_slow() -> String {
    format!(
        "the system did not hand the document over within {} s",
        OPEN_TIME.as_secs()
    )
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The worker: the descriptor from the system, whether the file is still
/// being written, then its name in the cache.
fn open_and_place(
    open_fd: fn(&str) -> Option<i32>,
    uri: &str,
    path: &Path,
    size: Option<u64>,
    tx: &Sender<Said>,
    abandoned: &Receiver<()>,
) -> Result<Placed, Failure> {
    let fd = open_fd(uri).ok_or_else(|| Failure::Failed("the document cannot be opened".into()))?;
    let file = owned(fd)?;
    if abandoned_now(abandoned) {
        return Err(Failure::Failed("given up on".into()));
    }
    ready(&file, size)?;
    place(file, path, tx, abandoned)
}

/// A plain path (the desktop preview, the emulator's test): whether it is
/// there and still being written; it is played where it is.
fn check_path(path: &Path, size: Option<u64>) -> Result<Placed, Failure> {
    let file = File::open(path)
        .map_err(|e| Failure::Failed(format!("the file cannot be opened ({e})")))?;
    ready(&file, size)?;
    Ok(Placed {
        path: path.to_owned(),
        file: None,
        copied: 0,
        own: false,
    })
}

/// Whether `file` can be played now: not still being written, not smaller
/// than the provider says it is.
fn ready(file: &File, size: Option<u64>) -> Result<(), Failure> {
    let length = file.metadata().map(|m| m.len()).unwrap_or(0);
    match still_written(file, SystemTime::now(), || std::thread::sleep(SECOND_LOOK)) {
        Some(Written::Copying(why)) => return Err(Failure::Copying(why, length)),
        Some(Written::Empty) => return Err(Failure::Failed("the file is empty".into())),
        None => {}
    }
    if let Some(size) = size.filter(|s| *s > 0)
        && file.metadata().is_ok_and(|m| m.is_file() && m.len() < size)
    {
        return Err(Failure::Copying(
            format!("still being copied ({length} of {size} bytes)"),
            length,
        ));
    }
    Ok(())
}

fn abandoned_now(abandoned: &Receiver<()>) -> bool {
    matches!(
        abandoned.try_recv(),
        Err(crossbeam_channel::TryRecvError::Disconnected)
    )
}

#[cfg(unix)]
fn owned(fd: i32) -> Result<File, Failure> {
    use std::os::fd::FromRawFd;
    if fd < 0 {
        return Err(Failure::Failed("the document cannot be opened".into()));
    }
    // Safety: the descriptor was opened for this call and is owned from
    // here on (the Java side detached it; a test hands over one it made).
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(not(unix))]
fn owned(_fd: i32) -> Result<File, Failure> {
    Err(Failure::Failed(
        "documents can only be opened on Android".into(),
    ))
}

/// What [`still_written`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Written {
    /// Still being written: why it seems so.
    Copying(String),
    /// Empty, and not changed for a while: not a copy under way.
    Empty,
}

/// Whether `file` is still being written (None: it is not, as far as can
/// be told). Only a regular file is judged: a pipe has no size to look at.
/// `pause` waits between the two looks.
pub fn still_written(file: &File, now: SystemTime, pause: impl FnOnce()) -> Option<Written> {
    let first = file.metadata().ok()?;
    if !first.is_file() {
        return None;
    }
    let age = first
        .modified()
        .ok()
        .and_then(|m| now.duration_since(m).ok())
        .unwrap_or(Duration::MAX);
    if first.len() == 0 {
        // (a copy that has just begun, or a file that is simply empty)
        return Some(if age < SETTLING {
            Written::Copying("still being copied (empty so far)".into())
        } else {
            Written::Empty
        });
    }
    if age < RECENT {
        return Some(Written::Copying(format!(
            "still being copied (changed {:.1} s ago)",
            age.as_secs_f32()
        )));
    }
    if age < SETTLING {
        pause();
        let second = file.metadata().ok()?;
        if second.len() != first.len() || second.modified().ok() != first.modified().ok() {
            return Some(Written::Copying("still being copied (it grows)".into()));
        }
    }
    None
}

/// Gives the open `file` the name `path`: a link to it where that can be
/// opened (the descriptor is kept), else a copy.
#[cfg(unix)]
fn place(
    file: File,
    path: &Path,
    tx: &Sender<Said>,
    abandoned: &Receiver<()>,
) -> Result<Placed, Failure> {
    use std::os::fd::AsRawFd;
    let _ = std::fs::remove_file(path);
    let target = format!("/proc/self/fd/{}", file.as_raw_fd());
    let linked = std::os::unix::fs::symlink(&target, path).is_ok()
        && std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
        && File::open(path).is_ok();
    if linked {
        return Ok(Placed {
            path: path.to_owned(),
            file: Some(file),
            copied: 0,
            own: true,
        });
    }
    let _ = std::fs::remove_file(path);
    let _ = tx.send(Said::Copying);
    match copy(
        file,
        path,
        COPY_BYTES,
        Instant::now() + COPY_TIME,
        abandoned,
    ) {
        Ok(bytes) if bytes > 0 => Ok(Placed {
            path: path.to_owned(),
            file: None,
            copied: bytes,
            own: true,
        }),
        Ok(_) => {
            let _ = std::fs::remove_file(path);
            Err(Failure::Failed("the document is empty".into()))
        }
        Err(e) => {
            let _ = std::fs::remove_file(path);
            Err(e)
        }
    }
}

#[cfg(not(unix))]
fn place(
    _file: File,
    _path: &Path,
    _tx: &Sender<Said>,
    _abandoned: &Receiver<()>,
) -> Result<Placed, Failure> {
    Err(Failure::Failed(
        "documents can only be opened on Android".into(),
    ))
}

/// Copies `from` into a new file at `to`: at most `bytes`, until
/// `deadline`, and not on once no one waits for it.
fn copy(
    mut from: File,
    to: &Path,
    bytes: u64,
    deadline: Instant,
    abandoned: &Receiver<()>,
) -> Result<u64, Failure> {
    use std::io::{Read, Write};
    let fail = |e: std::io::Error| Failure::Failed(format!("the document cannot be read ({e})"));
    let mut out = File::create(to).map_err(fail)?;
    let mut buffer = vec![0u8; 256 * 1024];
    let mut copied = 0u64;
    loop {
        if Instant::now() > deadline {
            return Err(Failure::Failed(format!(
                "copying the document took longer than {} s",
                COPY_TIME.as_secs()
            )));
        }
        if abandoned_now(abandoned) {
            return Err(Failure::Failed("given up on".into()));
        }
        let read = match from.read(&mut buffer) {
            Ok(0) => return Ok(copied),
            Ok(read) => read,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(fail(e)),
        };
        copied += read as u64;
        if copied > bytes {
            return Err(Failure::Failed(format!(
                "the document is larger than {} MiB",
                bytes >> 20
            )));
        }
        out.write_all(&buffer[..read]).map_err(fail)?;
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::IntoRawFd;

    /// A folder of this test's own under the system's temporary one.
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("axrossamp-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Stands in for the Java side: "content://test/<path>" is the file at
    /// <path>, opened and handed over as a bare descriptor.
    fn open_fd(uri: &str) -> Option<i32> {
        let path = uri.strip_prefix("content://test")?;
        File::open(path).ok().map(IntoRawFd::into_raw_fd)
    }

    /// A file written a while ago (not one still being copied).
    fn settled(path: &Path, content: &[u8]) {
        let mut file = File::create(path).unwrap();
        file.write_all(content).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(600))
            .unwrap();
    }

    fn uri(path: &Path) -> String {
        format!("content://test{}", path.display())
    }

    #[test]
    fn a_plain_path_is_itself_and_is_never_removed() {
        let dir = scratch("plain");
        let track = dir.join("a.flac");
        settled(&track, b"fLaC and so on");
        let mut opener = Opener::new(None, open_fd);
        let path = track.to_string_lossy().into_owned();
        assert_eq!(opener.open(&path, "a.flac").unwrap(), path);
        assert_eq!(opener.kept(), 0);
        // (given up on: the user's file stays)
        let Begun::Opening(opening) = opener.begin(&path, "a.flac", None) else {
            panic!("not opening");
        };
        opening.abandon();
        std::thread::sleep(Duration::from_millis(200));
        assert!(track.exists());
        // (missing, or still being written: said so)
        assert!(matches!(
            opener.open("/no/such/file.mp3", "file.mp3"),
            Err(Failure::Failed(_))
        ));
        let fresh = dir.join("fresh.mp3");
        File::create(&fresh).unwrap().write_all(b"ID3").unwrap();
        assert!(matches!(
            opener.open(&fresh.to_string_lossy(), "fresh.mp3"),
            Err(Failure::Copying(_, 3))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_document_gets_a_name_the_engine_can_open() {
        let dir = scratch("link");
        let music = dir.join("music");
        std::fs::create_dir_all(&music).unwrap();
        let track = music.join("one.flac");
        settled(&track, b"fLaC and so on");
        let cache = dir.join("open");
        let mut opener = Opener::new(Some(cache.clone()), open_fd);

        let path = opener.open(&uri(&track), "One.FLAC").unwrap();
        // (in the cache, with the name's ending for the decoder's hint)
        assert!(path.starts_with(&*cache.to_string_lossy()), "{path}");
        assert!(path.ends_with(".flac"), "{path}");
        assert_eq!(std::fs::read(&path).unwrap(), b"fLaC and so on");
        // (opened once: asked again it is the same name)
        assert_eq!(opener.open(&uri(&track), "One.FLAC").unwrap(), path);
        assert_eq!(opener.kept(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_cannot_be_opened_says_so() {
        let dir = scratch("missing");
        let mut opener = Opener::new(Some(dir.join("open")), open_fd);
        assert!(matches!(
            opener.open("content://test/no/such/file.mp3", "file.mp3"),
            Err(Failure::Failed(_))
        ));
        // (and without a cache folder no document at all)
        let mut opener = Opener::new(None, open_fd);
        assert!(opener.open("content://test/x.mp3", "x.mp3").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_last_few_stay_open() {
        let dir = scratch("keep");
        let music = dir.join("music");
        std::fs::create_dir_all(&music).unwrap();
        let cache = dir.join("open");
        let mut opener = Opener::new(Some(cache.clone()), open_fd);
        let mut paths = Vec::new();
        for i in 0..KEEP + 3 {
            let track = music.join(format!("{i}.mp3"));
            settled(&track, b"ID3 and so on");
            paths.push(opener.open(&uri(&track), &format!("{i}.mp3")).unwrap());
        }
        assert_eq!(opener.kept(), KEEP);
        // (the oldest are gone from the cache, the newest readable)
        assert!(!Path::new(&paths[0]).exists());
        assert!(std::fs::symlink_metadata(&paths[2]).is_err());
        assert_eq!(
            std::fs::read(paths.last().unwrap()).unwrap(),
            b"ID3 and so on"
        );
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), KEEP);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_descriptor_that_cannot_be_opened_again_is_copied() {
        // (a pipe: /proc/self/fd/N of it is no regular file)
        let dir = scratch("pipe");
        let path = dir.join("1.ogg");
        let (reader, mut writer) = std::io::pipe().unwrap();
        writer.write_all(b"OggS and so on").unwrap();
        drop(writer);
        let (tx, said) = crossbeam_channel::unbounded();
        let (_keep, abandoned) = crossbeam_channel::bounded::<()>(0);
        let file = owned(reader.into_raw_fd()).unwrap();
        let placed = place(file, &path, &tx, &abandoned).unwrap();
        assert!(placed.file.is_none());
        assert_eq!(placed.copied, 14);
        assert!(placed.own);
        assert!(matches!(said.try_recv(), Ok(Said::Copying)));
        assert!(std::fs::symlink_metadata(&path).unwrap().is_file());
        assert_eq!(std::fs::read(&path).unwrap(), b"OggS and so on");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_still_being_copied_is_not_opened() {
        let dir = scratch("copying");
        let mut opener = Opener::new(Some(dir.join("open")), open_fd);
        // (empty, as a copy is when it starts)
        let empty = dir.join("empty.flac");
        File::create(&empty).unwrap();
        assert!(matches!(
            opener.open(&uri(&empty), "empty.flac"),
            Err(Failure::Copying(_, 0))
        ));
        // (empty for a while: no copy under way, a failure)
        File::options()
            .write(true)
            .open(&empty)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(600))
            .unwrap();
        assert!(matches!(
            opener.open(&uri(&empty), "empty.flac"),
            Err(Failure::Failed(m)) if m.contains("empty")
        ));
        // (written to a moment ago)
        let fresh = dir.join("fresh.mp3");
        File::create(&fresh)
            .unwrap()
            .write_all(b"ID3 half")
            .unwrap();
        assert!(matches!(
            opener.open(&uri(&fresh), "fresh.mp3"),
            Err(Failure::Copying(_, 8))
        ));
        // (a while ago: a whole file)
        settled(&fresh, b"ID3 whole");
        assert!(opener.open(&uri(&fresh), "fresh.mp3").is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_grows_between_two_looks_is_still_being_copied() {
        let dir = scratch("grows");
        let path = dir.join("growing.mp3");
        // (last changed half a minute ago - then it grows while looked at)
        let mut writer = File::create(&path).unwrap();
        writer.write_all(b"ID3 begun").unwrap();
        writer
            .set_modified(SystemTime::now() - Duration::from_secs(30))
            .unwrap();
        let reader = File::open(&path).unwrap();
        let why = still_written(&reader, SystemTime::now(), || {
            writer.write_all(b" and more").unwrap();
        });
        assert!(matches!(why, Some(Written::Copying(w)) if w.contains("grows")));
        // (the same, untouched between the looks: done)
        let settled_at = SystemTime::now() - Duration::from_secs(30);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(settled_at)
            .unwrap();
        assert_eq!(still_written(&reader, SystemTime::now(), || {}), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A provider that does not answer: (a busy phone's) the opening is
    /// given up on at its deadline and cleans up when it is done.
    fn stuck(_: &str) -> Option<i32> {
        std::thread::sleep(Duration::from_millis(600));
        None
    }

    #[test]
    fn a_provider_that_does_not_answer_holds_nothing_up() {
        let dir = scratch("stuck");
        let mut opener = Opener::new(Some(dir.join("open")), stuck);
        let begun = Instant::now();
        let Begun::Opening(mut opening) = opener.begin("content://slow/a.mp3", "a.mp3", None)
        else {
            panic!("not opening");
        };
        // (beginning does not wait)
        assert!(begun.elapsed() < Duration::from_millis(100));
        assert!(opening.take().is_none());
        opening.abandon();
        // (the worker ends on its own; nothing is left in the cache)
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(std::fs::read_dir(dir.join("open")).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_stops_at_its_byte_budget_and_when_given_up_on() {
        let dir = scratch("budget");
        let from = dir.join("big.bin");
        settled(&from, &[7u8; 4096]);
        let (_keep, waiting) = crossbeam_channel::bounded::<()>(0);
        let to = dir.join("copy.bin");
        let far = Instant::now() + Duration::from_secs(30);
        assert_eq!(
            copy(File::open(&from).unwrap(), &to, 1 << 20, far, &waiting),
            Ok(4096)
        );
        assert!(matches!(
            copy(File::open(&from).unwrap(), &to, 1000, far, &waiting),
            Err(Failure::Failed(m)) if m.contains("larger")
        ));
        // (past its deadline)
        let past = Instant::now() - Duration::from_secs(1);
        assert!(copy(File::open(&from).unwrap(), &to, 1 << 20, past, &waiting).is_err());
        // (no one waits)
        let (gone, abandoned) = crossbeam_channel::bounded::<()>(0);
        drop(gone);
        assert!(copy(File::open(&from).unwrap(), &to, 1 << 20, far, &abandoned).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_smaller_than_the_provider_says_is_still_being_copied() {
        let dir = scratch("size");
        let track = dir.join("half.mp3");
        settled(&track, b"ID3 half");
        let mut opener = Opener::new(Some(dir.join("open")), open_fd);
        let Begun::Opening(mut opening) = opener.begin(&uri(&track), "half.mp3", Some(100)) else {
            panic!("not opening");
        };
        assert!(matches!(
            opening.wait(Duration::from_secs(5)),
            Some(Err(Failure::Copying(m, 8))) if m.contains("8 of 100")
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_track_playing_and_the_one_queued_are_never_let_go_of() {
        let dir = scratch("pinned");
        let music = dir.join("music");
        std::fs::create_dir_all(&music).unwrap();
        let mut opener = Opener::new(Some(dir.join("open")), open_fd);
        let open = |opener: &mut Opener, name: &str, in_use: &[&str]| {
            let track = music.join(name);
            settled(&track, name.as_bytes());
            let Begun::Opening(mut opening) = opener.begin(&uri(&track), name, None) else {
                panic!("not opening");
            };
            let placed = opening.wait(Duration::from_secs(5)).unwrap().unwrap();
            opener.finish(&uri(&track), placed, in_use)
        };
        let playing_uri = uri(&music.join("playing.mp3"));
        let queued_uri = uri(&music.join("queued.mp3"));
        let playing = open(&mut opener, "playing.mp3", &[]);
        let queued = open(&mut opener, "queued.mp3", &[&playing_uri]);
        // (one long track: ten others opened to follow it, one after another)
        for i in 0..10 {
            open(
                &mut opener,
                &format!("{i}.mp3"),
                &[&playing_uri, &queued_uri],
            );
        }
        assert_eq!(std::fs::read(&playing).unwrap(), b"playing.mp3");
        assert_eq!(std::fs::read(&queued).unwrap(), b"queued.mp3");
        assert_eq!(opener.kept(), KEEP);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_more_than_a_few_openings_wait_on_a_provider_at_once() {
        let dir = scratch("openings");
        let mut opener = Opener::new(Some(dir.join("open")), stuck);
        let mut waiting = Vec::new();
        let mut busy = 0;
        for i in 0..OPENINGS + 2 {
            match opener.begin(&format!("content://slow/{i}.mp3"), "x.mp3", None) {
                Begun::Opening(opening) => waiting.push(opening),
                Begun::Failed(Failure::Copying(m, _)) if m.contains("busy") => busy += 1,
                _ => panic!("unexpected"),
            }
        }
        assert_eq!((waiting.len(), busy), (OPENINGS, 2));
        drop(waiting);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
