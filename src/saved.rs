//! What the player keeps between starts: the tracks picked last (their
//! URIs - the app holds a persisted read permission for them, taken when
//! they were picked), the current one and where it was. At the next start
//! the list is there again, that track opened there, paused, ready to
//! play: nothing to pick again.
//!
//! The file is the app's own (its files folder), plain text: a first line
//! that names the format, then `index`, `position`, and one `track` line a
//! track in the pickers' own form (`picked::parse`). It is written to a new
//! file and renamed over the old one, so a process killed while writing
//! leaves the old state, never half of one.

use crate::picked::{self, Picked};
use std::path::Path;

/// The file's name in the app's files.
pub const FILE: &str = "player-state.txt";

const HEADER: &str = "axross-amp-android state 1";

/// The state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Saved {
    pub tracks: Vec<Picked>,
    /// The current track's place in `tracks`.
    pub index: usize,
    /// Seconds into it.
    pub position: f64,
}

impl Saved {
    /// As text (see the module).
    pub fn to_text(&self) -> String {
        let mut text = format!(
            "{HEADER}\nindex\t{}\nposition\t{:.3}\n",
            self.index,
            if self.position.is_finite() {
                self.position.max(0.0)
            } else {
                0.0
            }
        );
        for t in &self.tracks {
            let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
            text.push_str(&format!(
                "track\t{}\t{}\t{}\t{}\n",
                clean(&t.uri),
                clean(&t.folder),
                clean(&t.name),
                t.size.map_or(-1, |s| s as i64)
            ));
        }
        text
    }

    /// From text; None for anything that is not this format.
    pub fn from_text(text: &str) -> Option<Saved> {
        let mut lines = text.lines();
        if lines.next()? != HEADER {
            return None;
        }
        let mut saved = Saved::default();
        let mut tracks = String::new();
        for line in lines {
            let (key, rest) = line.split_once('\t').unwrap_or((line, ""));
            match key {
                "index" => saved.index = rest.trim().parse().ok()?,
                "position" => {
                    saved.position = rest
                        .trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|p| p.is_finite())?
                        .max(0.0)
                }
                "track" => {
                    tracks.push_str(rest);
                    tracks.push('\n');
                }
                // (a later version's lines: left alone)
                _ => {}
            }
        }
        saved.tracks = picked::parse(&tracks);
        if saved.index >= saved.tracks.len() {
            saved.index = 0;
            saved.position = 0.0;
        }
        Some(saved)
    }

    /// Written to `dir/FILE` (through a new file renamed over the old).
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let path = dir.join(FILE);
        let new = dir.join(format!("{FILE}.new"));
        std::fs::write(&new, self.to_text())?;
        std::fs::rename(&new, &path)
    }

    /// Read from `dir/FILE`: None when there is none or it cannot be read.
    pub fn load(dir: &Path) -> Option<Saved> {
        Saved::from_text(&std::fs::read_to_string(dir.join(FILE)).ok()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(uri: &str, name: &str, size: Option<u64>) -> Picked {
        Picked {
            uri: uri.into(),
            folder: "Album".into(),
            name: name.into(),
            size,
        }
    }

    #[test]
    fn the_state_comes_back_as_it_was() {
        let saved = Saved {
            tracks: vec![
                track("content://a/1", "01 One.flac", Some(1234)),
                track("content://a/2", "02 Two\tand a tab.mp3", None),
            ],
            index: 1,
            position: 83.25,
        };
        let back = Saved::from_text(&saved.to_text()).unwrap();
        assert_eq!(back.index, 1);
        assert_eq!(back.position, 83.25);
        assert_eq!(back.tracks.len(), 2);
        assert_eq!(back.tracks[0], saved.tracks[0]);
        assert_eq!(back.tracks[1].name, "02 Two and a tab.mp3");
        assert_eq!(back.tracks[1].size, None);
    }

    #[test]
    fn a_broken_or_foreign_file_is_no_state() {
        assert_eq!(Saved::from_text(""), None);
        assert_eq!(Saved::from_text("something else\nindex\t1\n"), None);
        assert_eq!(Saved::from_text(&format!("{HEADER}\nindex\tx\n")), None);
        assert_eq!(
            Saved::from_text(&format!("{HEADER}\nposition\tNaN\n")),
            None
        );
        // (an index past the list: the first track, from the start)
        let odd = Saved::from_text(&format!(
            "{HEADER}\nindex\t9\nposition\t20\ntrack\tu\t\ta.mp3\t-1\nnew-key\tx\n"
        ))
        .unwrap();
        assert_eq!((odd.index, odd.position, odd.tracks.len()), (0, 0.0, 1));
    }

    #[test]
    fn saved_and_loaded_through_a_file() {
        let dir = std::env::temp_dir().join(format!("axrossamp-saved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(Saved::load(&dir), None);
        let saved = Saved {
            tracks: vec![track("content://a/1", "1.mp3", None)],
            index: 0,
            position: 5.0,
        };
        saved.save(&dir).unwrap();
        assert_eq!(Saved::load(&dir), Some(saved));
        assert!(!dir.join(format!("{FILE}.new")).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
