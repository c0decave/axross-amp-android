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

struct Wanted {
    /// The music should be playing.
    playing: bool,
    /// It was paused for a moment by the system (a call): it goes on when
    /// the audio focus is back.
    resume: bool,
}

pub struct Transport {
    tx: Sender<Command>,
    wanted: Mutex<Wanted>,
}

impl Transport {
    pub fn new(tx: Sender<Command>) -> Transport {
        Transport {
            tx,
            wanted: Mutex::new(Wanted {
                playing: false,
                resume: false,
            }),
        }
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
        let _ = self.tx.send(Command::Play);
    }

    /// The user's pause.
    pub fn pause(&self) {
        let mut wanted = self.wanted();
        wanted.playing = false;
        wanted.resume = false;
        let _ = self.tx.send(Command::Pause);
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
        let _ = self.tx.send(Command::Load {
            path,
            start_ms,
            play: play && wanted.playing,
            generation,
        });
    }

    pub fn queue(&self, queued: Option<Queued>) {
        let _ = self.tx.send(Command::Queue(queued));
    }

    pub fn seek(&self, secs: f64) {
        let _ = self.tx.send(Command::Seek(secs));
    }

    pub fn stop(&self) {
        let mut wanted = self.wanted();
        wanted.playing = false;
        wanted.resume = false;
        let _ = self.tx.send(Command::Stop);
    }

    /// The system's word, acted on at once (on its thread).
    pub fn interrupt(&self, what: Interrupt) {
        let mut wanted = self.wanted();
        match what {
            Interrupt::Hidden | Interrupt::FocusLost | Interrupt::Noisy => {
                wanted.playing = false;
                wanted.resume = false;
                let _ = self.tx.send(Command::Pause);
            }
            Interrupt::FocusLostForNow => {
                wanted.resume |= wanted.playing;
                wanted.playing = false;
                let _ = self.tx.send(Command::Pause);
            }
            Interrupt::FocusBack => {
                if std::mem::take(&mut wanted.resume) {
                    wanted.playing = true;
                    let _ = self.tx.send(Command::Play);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::Receiver;

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
