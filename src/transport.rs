//! The engine's commands, sent from more than one thread: the window's
//! (the buttons), the one that opens documents ([`crate::source`]) and
//! whichever thread the system interrupts on ([`crate::platform`]).
//!
//! One thing must hold between them: music the user or the system paused
//! does not start again because a track that was still being opened arrives
//! after the pause. So "is the music wanted" is kept here, under a lock
//! with the sending: a load plays only if it still is.

use crate::amp::engine::{Command, Queued};
use crate::platform::Interrupt;
use crossbeam_channel::Sender;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

struct Wanted {
    /// The music should be playing.
    playing: bool,
    /// It was paused for a moment by the system (a call): it goes on when
    /// the audio focus is back.
    resume: bool,
}

pub struct Transport {
    /// The engine's commands (another engine's after a rest: see
    /// [`Transport::set_engine`]).
    tx: Mutex<Sender<Command>>,
    wanted: Mutex<Wanted>,
    /// Counts the pauses - the user's, the system's, a stop: what was
    /// planned to play later plays only if none came between.
    pauses: AtomicU64,
}

impl Transport {
    pub fn new(tx: Sender<Command>) -> Transport {
        Transport {
            tx: Mutex::new(tx),
            wanted: Mutex::new(Wanted {
                playing: false,
                resume: false,
            }),
            pauses: AtomicU64::new(0),
        }
    }

    fn send(&self, command: Command) {
        let tx = self
            .tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = tx.send(command);
    }

    /// The engine's commands go to `tx` from now on (a new engine).
    pub fn set_engine(&self, tx: Sender<Command>) {
        *self
            .tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = tx;
    }

    /// The engine ends (the app rests while hidden: its output stream is
    /// closed); a new one comes with [`Transport::set_engine`].
    pub fn quit(&self) {
        self.send(Command::Quit);
    }

    /// The pauses so far (see the field).
    pub fn pauses(&self) -> u64 {
        self.pauses.load(Ordering::Relaxed)
    }

    fn paused(&self) {
        self.pauses.fetch_add(1, Ordering::Relaxed);
    }

    fn wanted(&self) -> std::sync::MutexGuard<'_, Wanted> {
        self.wanted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether the music should be playing (it may still be opening).
    pub fn wants_playing(&self) -> bool {
        self.wanted().playing
    }

    /// The user's play.
    pub fn play(&self) {
        let mut wanted = self.wanted();
        wanted.playing = true;
        wanted.resume = false;
        self.send(Command::Play);
    }

    /// The user's pause.
    pub fn pause(&self) {
        let mut wanted = self.wanted();
        wanted.playing = false;
        wanted.resume = false;
        self.paused();
        self.send(Command::Pause);
    }

    /// A track is about to be opened to play (or not): what is wanted from
    /// now on, said before the opening so that a pause meanwhile counts.
    pub fn intend(&self, play: bool) {
        let mut wanted = self.wanted();
        wanted.playing = play;
        wanted.resume = false;
    }

    /// The track is open: it plays if that was asked and is still wanted.
    pub fn load(&self, path: String, start_ms: u64, play: bool, generation: u64) {
        let wanted = self.wanted();
        self.send(Command::Load {
            path,
            start_ms,
            play: play && wanted.playing,
            generation,
        });
    }

    pub fn queue(&self, queued: Option<Queued>) {
        self.send(Command::Queue(queued));
    }

    pub fn seek(&self, secs: f64) {
        self.send(Command::Seek(secs));
    }

    /// Anything else for the engine (another output: [`Command::Output`]).
    pub fn command(&self, command: Command) {
        self.send(command);
    }

    pub fn stop(&self) {
        let mut wanted = self.wanted();
        wanted.playing = false;
        wanted.resume = false;
        self.paused();
        self.send(Command::Stop);
    }

    /// The system's word, acted on at once (on its thread).
    pub fn interrupt(&self, what: Interrupt) {
        let mut wanted = self.wanted();
        match what {
            Interrupt::Hidden | Interrupt::FocusLost | Interrupt::Noisy => {
                wanted.playing = false;
                wanted.resume = false;
                self.paused();
                self.send(Command::Pause);
            }
            Interrupt::FocusLostForNow => {
                wanted.resume |= wanted.playing;
                wanted.playing = false;
                self.paused();
                self.send(Command::Pause);
            }
            Interrupt::FocusBack => {
                if std::mem::take(&mut wanted.resume) {
                    wanted.playing = true;
                    self.send(Command::Play);
                }
            }
        }
    }
}

/// How close to its end a wind (a long press, the seek bar) may take a
/// track: it does not end the track (a skip past the end does).
pub const WIND_END: f64 = 1.0;

/// Where a seek goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SeekTo {
    /// To this many seconds into the track.
    At(f64),
    /// On to the next track, as at the end of this one.
    Next,
    /// Nowhere (not a number).
    Nowhere,
}

/// A seek to `secs` in a track `duration` seconds long (None: not known
/// yet): from 0:00 at the least. At or past the end, `past_end` decides:
/// [`SeekTo::Next`] for a skip, or [`WIND_END`] before the end for a wind
/// and the seek bar (with no length known, a wind is not held back).
pub fn seek_to(secs: f64, duration: Option<f64>, past_end: PastEnd) -> SeekTo {
    if !secs.is_finite() {
        return SeekTo::Nowhere;
    }
    let secs = secs.max(0.0);
    match duration.filter(|d| d.is_finite() && *d > 0.0) {
        Some(duration) if secs >= duration => match past_end {
            PastEnd::Next => SeekTo::Next,
            PastEnd::Stay => SeekTo::At((duration - WIND_END).max(0.0)),
        },
        Some(duration) if past_end == PastEnd::Stay => {
            SeekTo::At(secs.min((duration - WIND_END).max(0.0)))
        }
        _ => SeekTo::At(secs),
    }
}

/// What a seek at or past the end of a track does (see [`seek_to`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PastEnd {
    Next,
    Stay,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::Receiver;

    #[test]
    fn a_seek_before_the_start_is_the_start() {
        assert_eq!(seek_to(-15.0, Some(200.0), PastEnd::Next), SeekTo::At(0.0));
        assert_eq!(seek_to(-3.0, None, PastEnd::Stay), SeekTo::At(0.0));
        assert_eq!(seek_to(42.5, Some(200.0), PastEnd::Next), SeekTo::At(42.5));
    }

    #[test]
    fn a_skip_past_the_end_is_the_next_track_a_wind_stops_before_it() {
        assert_eq!(seek_to(205.0, Some(200.0), PastEnd::Next), SeekTo::Next);
        assert_eq!(seek_to(200.0, Some(200.0), PastEnd::Next), SeekTo::Next);
        assert_eq!(
            seek_to(205.0, Some(200.0), PastEnd::Stay),
            SeekTo::At(199.0)
        );
        assert_eq!(
            seek_to(199.5, Some(200.0), PastEnd::Stay),
            SeekTo::At(199.0)
        );
        // (a track shorter than the margin)
        assert_eq!(seek_to(3.0, Some(0.5), PastEnd::Stay), SeekTo::At(0.0));
    }

    #[test]
    fn without_a_length_only_the_start_holds_a_seek_back() {
        // (still opening, or a length nobody knows)
        assert_eq!(seek_to(500.0, None, PastEnd::Next), SeekTo::At(500.0));
        assert_eq!(
            seek_to(500.0, Some(f64::NAN), PastEnd::Stay),
            SeekTo::At(500.0)
        );
        assert_eq!(seek_to(500.0, Some(0.0), PastEnd::Next), SeekTo::At(500.0));
        assert_eq!(
            seek_to(f64::NAN, Some(200.0), PastEnd::Next),
            SeekTo::Nowhere
        );
        assert_eq!(seek_to(f64::INFINITY, None, PastEnd::Next), SeekTo::Nowhere);
    }

    fn transport() -> (Transport, Receiver<Command>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        (Transport::new(tx), rx)
    }

    /// The commands sent so far, by name (and whether a load plays).
    fn sent(rx: &Receiver<Command>) -> Vec<String> {
        rx.try_iter()
            .map(|c| match c {
                Command::Play => "play".to_owned(),
                Command::Pause => "pause".to_owned(),
                Command::Stop => "stop".to_owned(),
                Command::Load { play, .. } => format!("load play={play}"),
                Command::Queue(q) => format!("queue {}", q.is_some()),
                Command::Seek(_) => "seek".to_owned(),
                _ => "other".to_owned(),
            })
            .collect()
    }

    #[test]
    fn a_track_opened_after_a_pause_does_not_play() {
        let (t, rx) = transport();
        t.intend(true);
        // (the app is hidden while the document is still being opened)
        t.interrupt(Interrupt::Hidden);
        t.load("a".into(), 0, true, 1);
        assert_eq!(sent(&rx), ["pause", "load play=false"]);
        assert!(!t.wants_playing());
    }

    #[test]
    fn a_track_opened_while_wanted_plays() {
        let (t, rx) = transport();
        t.intend(true);
        t.load("a".into(), 0, true, 1);
        // (opened to stand at its start: not played though music is wanted)
        t.load("b".into(), 0, false, 2);
        assert_eq!(sent(&rx), ["load play=true", "load play=false"]);
    }

    #[test]
    fn a_moment_without_focus_ends_where_it_began() {
        let (t, rx) = transport();
        t.play();
        t.interrupt(Interrupt::FocusLostForNow);
        assert!(!t.wants_playing());
        t.interrupt(Interrupt::FocusBack);
        assert!(t.wants_playing());
        assert_eq!(sent(&rx), ["play", "pause", "play"]);
    }

    #[test]
    fn focus_coming_back_does_not_start_what_the_user_paused() {
        let (t, rx) = transport();
        t.play();
        t.interrupt(Interrupt::FocusLostForNow);
        // (the user pauses during the call: that stands)
        t.pause();
        t.interrupt(Interrupt::FocusBack);
        assert!(!t.wants_playing());
        assert_eq!(sent(&rx), ["play", "pause", "pause"]);
        // (nor music that was not playing when the focus went)
        t.interrupt(Interrupt::FocusLostForNow);
        t.interrupt(Interrupt::FocusBack);
        assert_eq!(sent(&rx), ["pause"]);
    }

    #[test]
    fn focus_lost_for_good_or_the_headphones_gone_stays_paused() {
        let (t, rx) = transport();
        t.play();
        t.interrupt(Interrupt::FocusLostForNow);
        t.interrupt(Interrupt::FocusLost);
        t.interrupt(Interrupt::FocusBack);
        t.play();
        t.interrupt(Interrupt::Noisy);
        t.interrupt(Interrupt::FocusBack);
        assert!(!t.wants_playing());
        assert_eq!(sent(&rx), ["play", "pause", "pause", "play", "pause"]);
    }
}
