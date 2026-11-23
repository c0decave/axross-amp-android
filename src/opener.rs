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
//!   into the cache instead.
//!
//! Only the last few tracks stay open ([`KEEP`]): the one playing, the one
//! queued behind it, the ones just left.

use crate::picked;
use std::collections::VecDeque;
use std::fs::File;
use std::path::{Path, PathBuf};

/// How many opened tracks are kept (their descriptors, links or copies).
pub const KEEP: usize = 6;

/// Whether `uri` is a document of a content provider (else: a path).
pub fn is_document(uri: &str) -> bool {
    uri.starts_with("content://")
}

struct Open {
    uri: String,
    path: PathBuf,
    /// The descriptor the link points at (None: a copy).
    _file: Option<File>,
}

pub struct Opener {
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
            dir,
            open_fd,
            open: VecDeque::new(),
            count: 0,
        }
    }

    /// The path the engine opens `uri` under.
    pub fn open(&mut self, uri: &str, name: &str) -> Result<String, String> {
        if !is_document(uri) {
            return Ok(uri.to_owned());
        }
        if let Some(at) = self.open.iter().position(|o| o.uri == uri) {
            // (used again: the last to go)
            let again = self.open.remove(at).expect("the place just found");
            let path = path_text(&again.path);
            self.open.push_back(again);
            return Ok(path);
        }
        let dir = self.dir.clone().ok_or("no place to open documents in")?;
        let fd = (self.open_fd)(uri).ok_or("the document cannot be opened")?;
        self.count += 1;
        let ending = picked::extension(name).unwrap_or_else(|| "bin".to_owned());
        let path = dir.join(format!("{}.{ending}", self.count));
        let file = place(fd, &path)?;
        self.open.push_back(Open {
            uri: uri.to_owned(),
            path: path.clone(),
            _file: file,
        });
        while self.open.len() > KEEP {
            if let Some(old) = self.open.pop_front() {
                let _ = std::fs::remove_file(&old.path);
            }
        }
        Ok(path_text(&path))
    }

    /// How many tracks are open now.
    pub fn kept(&self) -> usize {
        self.open.len()
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Gives the open descriptor `fd` the name `path`: a link to it where that
/// can be opened (the descriptor is kept: Some), else a copy (None).
#[cfg(unix)]
fn place(fd: i32, path: &Path) -> Result<Option<File>, String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    // Safety: the descriptor was opened for this call and is owned from
    // here on (the Java side detached it; a test hands over one it made).
    let file = unsafe { File::from_raw_fd(fd) };
    let _ = std::fs::remove_file(path);
    let target = format!("/proc/self/fd/{}", file.as_raw_fd());
    let linked = std::os::unix::fs::symlink(&target, path).is_ok()
        && std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
        && File::open(path).is_ok();
    if linked {
        return Ok(Some(file));
    }
    let _ = std::fs::remove_file(path);
    let mut from = file;
    let copied = File::create(path)
        .and_then(|mut to| std::io::copy(&mut from, &mut to))
        .map_err(|e| format!("the document cannot be read ({e})"));
    match copied {
        Ok(bytes) if bytes > 0 => Ok(None),
        Ok(_) => {
            let _ = std::fs::remove_file(path);
            Err("the document is empty".into())
        }
        Err(e) => {
            let _ = std::fs::remove_file(path);
            Err(e)
        }
    }
}

#[cfg(not(unix))]
fn place(_fd: i32, _path: &Path) -> Result<Option<File>, String> {
    Err("documents can only be opened on Android".into())
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

    #[test]
    fn a_plain_path_is_itself() {
        let mut opener = Opener::new(None, open_fd);
        assert_eq!(
            opener.open("/music/a.flac", "a.flac").unwrap(),
            "/music/a.flac"
        );
        assert_eq!(opener.kept(), 0);
    }

    #[test]
    fn a_document_gets_a_name_the_engine_can_open() {
        let dir = scratch("link");
        let music = dir.join("music");
        std::fs::create_dir_all(&music).unwrap();
        let track = music.join("one.flac");
        File::create(&track)
            .unwrap()
            .write_all(b"fLaC and so on")
            .unwrap();
        let cache = dir.join("open");
        let mut opener = Opener::new(Some(cache.clone()), open_fd);

        let uri = format!("content://test{}", track.display());
        let path = opener.open(&uri, "One.FLAC").unwrap();
        // (in the cache, with the name's ending for the decoder's hint)
        assert!(path.starts_with(&*cache.to_string_lossy()), "{path}");
        assert!(path.ends_with(".flac"), "{path}");
        assert_eq!(std::fs::read(&path).unwrap(), b"fLaC and so on");
        // (opened once: asked again it is the same name)
        assert_eq!(opener.open(&uri, "One.FLAC").unwrap(), path);
        assert_eq!(opener.kept(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_cannot_be_opened_says_so() {
        let dir = scratch("missing");
        let mut opener = Opener::new(Some(dir.join("open")), open_fd);
        assert!(
            opener
                .open("content://test/no/such/file.mp3", "file.mp3")
                .is_err()
        );
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
            File::create(&track)
                .unwrap()
                .write_all(b"ID3 and so on")
                .unwrap();
            let uri = format!("content://test{}", track.display());
            paths.push(opener.open(&uri, &format!("{i}.mp3")).unwrap());
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
        let kept = place(reader.into_raw_fd(), &path).unwrap();
        assert!(kept.is_none());
        assert!(std::fs::symlink_metadata(&path).unwrap().is_file());
        assert_eq!(std::fs::read(&path).unwrap(), b"OggS and so on");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
