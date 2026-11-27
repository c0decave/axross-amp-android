//! The seam between the player and Android: one way out ([`call`], the
//! activity's `bridgeCall`) and one way in ([`dispatch`], `Bridge.nativeEvent`).
//! The numbers are the same as in `Bridge.java`. On the host both ends are
//! stand-ins ([`host`]): the desktop preview and the tests run without a
//! device.
//!
//! Events arrive on Java's threads while the window may not be drawing at
//! all (the activity stopped). What must not wait for a frame - the music
//! paused when the app is hidden, when another app takes the audio focus,
//! when the headphones are pulled out - is done at once by the interrupt
//! hook; everything else is queued for the next frame.

use crate::picked::{self, Picked};
use std::sync::Mutex;

#[cfg(target_os = "android")]
pub mod android;
#[cfg(target_os = "android")]
use android as imp;

#[cfg(not(target_os = "android"))]
pub mod host;
#[cfg(not(target_os = "android"))]
use host as imp;

// Rust -> Java (Bridge.call)
pub const CALL_PICK_FOLDER: i32 = 1;
pub const CALL_PICK_FILES: i32 = 2;
pub const CALL_OPEN_FD: i32 = 3;
pub const CALL_REQUEST_FOCUS: i32 = 4;
pub const CALL_ABANDON_FOCUS: i32 = 5;
pub const CALL_LEAVE: i32 = 6;
/// The screen kept on ("1") or not ("0").
pub const CALL_SCREEN_ON: i32 = 7;
/// The player's state for the media session, its notification and the
/// playback service (see [`Playback`]).
pub const CALL_PLAYBACK: i32 = 8;

// Java -> Rust (Bridge.event)
pub const EVENT_INSETS: i32 = 1;
pub const EVENT_PICKED: i32 = 2;
pub const EVENT_LIFECYCLE: i32 = 3;
pub const EVENT_AUDIO_FOCUS: i32 = 4;
pub const EVENT_NOISY: i32 = 5;
/// Reserved for step 2: a MediaSession's callbacks (lock screen, headset,
/// notification) - `a` is one of the `MEDIA_*` commands.
pub const EVENT_MEDIA: i32 = 6;
/// The Java side's watchdog: the main thread has been blocked `a` ms (the
/// Rust side's steps go to the log at once).
pub const EVENT_STALL: i32 = 7;
/// A line to show (`text`): how the app ended last time, if it crashed.
pub const EVENT_NOTE: i32 = 8;

pub const MEDIA_PLAY: i32 = 1;
pub const MEDIA_PAUSE: i32 = 2;
pub const MEDIA_TOGGLE: i32 = 3;
pub const MEDIA_NEXT: i32 = 4;
pub const MEDIA_PREVIOUS: i32 = 5;
pub const MEDIA_STOP: i32 = 6;
/// Fast forward and rewind (a headset's, the lock screen's: step 2's
/// MediaSession) - the same skips as the buttons.
pub const MEDIA_FORWARD: i32 = 7;
pub const MEDIA_REWIND: i32 = 8;
/// To `b` milliseconds into the track (the notification's, the lock
/// screen's seek bar).
pub const MEDIA_SEEK_TO: i32 = 9;

// android.media.AudioManager's AUDIOFOCUS_* values
const FOCUS_GAIN: i32 = 1;
const FOCUS_LOSS: i32 = -1;
const FOCUS_LOSS_TRANSIENT: i32 = -2;
const FOCUS_LOSS_TRANSIENT_CAN_DUCK: i32 = -3;

/// What the system did that the music must follow at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interrupt {
    /// The activity is no longer shown (no playback service yet).
    Hidden,
    /// Another app plays now, for good.
    FocusLost,
    /// Another app plays for a moment (a call, a navigation prompt).
    FocusLostForNow,
    /// The focus is back after [`Interrupt::FocusLostForNow`].
    FocusBack,
    /// The headphones were pulled out or the Bluetooth speaker went away.
    Noisy,
}

/// What the next frame is told.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The window's safe insets, in pixels: left, top, right, bottom.
    Insets([i32; 4]),
    /// A batch of a picker's answer: a folder's tracks come in batches
    /// while it is read. `pick` counts the picks (a batch of an older one
    /// is of no use), `more` says whether more batches follow, `total` how
    /// many tracks there are so far (an empty last batch with none:
    /// nothing chosen, or nothing playable in it).
    Picked {
        tracks: Vec<Picked>,
        pick: i32,
        more: bool,
        total: i32,
    },
    /// The device has no documents app to pick with.
    NoPicker,
    /// The activity is shown again or hidden.
    Shown(bool),
    /// Told for the picture's sake; the music was already seen to.
    Interrupted(Interrupt),
    /// A line to show (how the app ended last time).
    Note(String),
}

/// What goes to the player at once, on its own thread (see
/// [`set_player_hook`]): it plays on while no window is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToPlayer {
    /// A media command (the notification's, the lock screen's, a headset's,
    /// Bluetooth's): one of the `MEDIA_*`, with its argument.
    Media(i32, i32),
    /// The answer to [`ask_audio_focus`]: whether the music may play.
    Focus(bool),
}

/// The player's state as the system shows it (the media session, its
/// notification, the lock screen) and keeps the playback service for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playback {
    /// 0 nothing to play, 1 stopped, 2 paused, 3 playing (as
    /// android.media.session.PlaybackState counts).
    pub state: i32,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
}

impl Playback {
    /// The line the Java side reads (Playback.update).
    pub fn to_line(&self) -> String {
        let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            self.state,
            self.position_ms,
            self.duration_ms,
            clean(&self.title),
            clean(&self.artist),
            clean(&self.album)
        )
    }
}

type Waker = Box<dyn Fn() + Send>;
type InterruptHook = Box<dyn Fn(Interrupt) + Send>;
type PlayerHook = Box<dyn Fn(ToPlayer) + Send>;
type LifecycleHook = Box<dyn Fn(bool) + Send>;

struct Hub {
    events: Vec<Event>,
    waker: Option<Waker>,
    interrupt: Option<InterruptHook>,
    player: Option<PlayerHook>,
    /// What came for the player before it was there (a media button that
    /// started the app).
    early: Vec<ToPlayer>,
    lifecycle: Option<LifecycleHook>,
}

static HUB: Mutex<Hub> = Mutex::new(Hub {
    events: Vec::new(),
    waker: None,
    interrupt: None,
    player: None,
    early: Vec::new(),
    lifecycle: None,
});

/// Called at once, on the Java side's thread, when the activity is shown
/// (true) or hidden: while hidden no frame comes to read an event.
pub fn set_lifecycle_hook(hook: impl Fn(bool) + Send + 'static) {
    hub().lifecycle = Some(Box::new(hook));
}

/// The tests that use the process's hub one after another.
#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());

/// No player any more (the tests).
#[cfg(test)]
pub fn clear_player_hook() {
    let mut hub = hub();
    hub.player = None;
    hub.early.clear();
}

/// Called at once with what is for the player (its thread's channel).
pub fn set_player_hook(hook: impl Fn(ToPlayer) + Send + 'static) {
    let mut hub = hub();
    for early in std::mem::take(&mut hub.early) {
        hook(early);
    }
    hub.player = Some(Box::new(hook));
}

fn to_player(what: ToPlayer) {
    let mut hub = hub();
    match hub.player.as_ref() {
        Some(hook) => hook(what),
        None => hub.early.push(what),
    }
}

fn hub() -> std::sync::MutexGuard<'static, Hub> {
    HUB.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Called when an event is waiting: the window draws a frame (if it draws).
pub fn set_waker(waker: impl Fn() + Send + 'static) {
    hub().waker = Some(Box::new(waker));
}

/// Called at once, on the thread the system told it on: it must not block
/// (it sends the engine a command).
pub fn set_interrupt_hook(hook: impl Fn(Interrupt) + Send + 'static) {
    hub().interrupt = Some(Box::new(hook));
}

/// The events since the last call.
pub fn poll() -> Vec<Event> {
    std::mem::take(&mut hub().events)
}

/// An event from the Java side (`Bridge.nativeEvent`), on whatever thread
/// it comes.
pub fn dispatch(kind: i32, a: i32, b: i32, c: i32, d: i32, text: &str) {
    let (event, interrupt) = match kind {
        EVENT_INSETS => (
            Event::Insets([a.max(0), b.max(0), c.max(0), d.max(0)]),
            None,
        ),
        EVENT_PICKED if a < 0 => (Event::NoPicker, None),
        EVENT_PICKED => (
            Event::Picked {
                tracks: picked::parse(text),
                more: b != 0,
                pick: c,
                total: d,
            },
            None,
        ),
        EVENT_STALL => {
            // (on the watchdog's thread, at once: the next frame may be the
            // one that does not come)
            let steps = crate::watch::report();
            log_line(&format!(
                "the main thread has been blocked for {a} ms; the app's own threads: {}",
                if steps.is_empty() { "all idle" } else { &steps }
            ));
            return;
        }
        // (hidden, the music plays on: the playback service keeps it; who
        // must know is told at once)
        EVENT_LIFECYCLE => {
            if let Some(hook) = hub().lifecycle.as_ref() {
                hook(a != 0);
            }
            (Event::Shown(a != 0), None)
        }
        EVENT_AUDIO_FOCUS => {
            let what = match a {
                FOCUS_GAIN => Interrupt::FocusBack,
                FOCUS_LOSS => Interrupt::FocusLost,
                FOCUS_LOSS_TRANSIENT => Interrupt::FocusLostForNow,
                // (the system lowers the volume by itself since Android 8)
                FOCUS_LOSS_TRANSIENT_CAN_DUCK => return,
                _ => return,
            };
            (Event::Interrupted(what), Some(what))
        }
        EVENT_NOISY => (Event::Interrupted(Interrupt::Noisy), Some(Interrupt::Noisy)),
        EVENT_MEDIA => return to_player(ToPlayer::Media(a, b)),
        EVENT_NOTE => (Event::Note(text.to_owned()), None),
        _ => return,
    };
    {
        let hub = hub();
        if let (Some(what), Some(hook)) = (interrupt, hub.interrupt.as_ref()) {
            hook(what);
        }
    }
    push(event);
}

/// The system's folder picker; the answer comes as [`Event::Picked`].
pub fn pick_folder() {
    imp::call(CALL_PICK_FOLDER, "");
}

/// The system's file picker (audio, several at once).
pub fn pick_files() {
    imp::call(CALL_PICK_FILES, "");
}

/// A content:// document opened for reading: a file descriptor the caller
/// owns, or None.
pub fn open_fd(uri: &str) -> Option<i32> {
    let fd = imp::call(CALL_OPEN_FD, uri);
    (fd >= 0).then_some(fd)
}

/// The queued event, and a frame to read it.
fn push(event: Event) {
    let mut hub = hub();
    hub.events.push(event);
    if let Some(wake) = hub.waker.as_ref() {
        wake();
    }
}

/// What the system is asked on [`system_worker`]'s thread.
enum Ask {
    Focus,
    AbandonFocus,
    ScreenOn(bool),
    Playback(String),
}

/// The calls that go to the system - the audio focus (binder), the
/// window's flags (the activity's glue) - are made on a thread of their
/// own: the window's thread, which Android's lifecycle and input wait on,
/// waits for nobody.
fn system_worker() -> &'static crossbeam_channel::Sender<Ask> {
    static WORKER: std::sync::OnceLock<crossbeam_channel::Sender<Ask>> = std::sync::OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = crossbeam_channel::unbounded::<Ask>();
        let _ = std::thread::Builder::new()
            .name("amp-system".into())
            .spawn(move || {
                for ask in rx {
                    match ask {
                        Ask::Focus => {
                            crate::watch::doing("amp-system", "asking for the audio focus");
                            // (1: granted; 0 refused; -1: the call failed -
                            // not granted either)
                            let granted = imp::call(CALL_REQUEST_FOCUS, "") == 1;
                            crate::watch::done("amp-system");
                            to_player(ToPlayer::Focus(granted));
                        }
                        Ask::AbandonFocus => {
                            crate::watch::doing("amp-system", "giving the audio focus up");
                            imp::call(CALL_ABANDON_FOCUS, "");
                            crate::watch::done("amp-system");
                        }
                        Ask::Playback(line) => {
                            crate::watch::doing("amp-system", "telling the media session");
                            imp::call(CALL_PLAYBACK, &line);
                            crate::watch::done("amp-system");
                        }
                        Ask::ScreenOn(on) => {
                            crate::watch::doing("amp-system", "setting the window's flags");
                            imp::keep_screen_on(on);
                            crate::watch::done("amp-system");
                        }
                    }
                }
            });
        tx
    })
}

/// Asks for the audio focus (the music is about to play): the answer goes
/// to the player ([`ToPlayer::Focus`]) - false: another app holds it and
/// does not give it up (a call).
pub fn ask_audio_focus() {
    let _ = system_worker().send(Ask::Focus);
}

/// Gives the audio focus up (the music paused by the user).
pub fn abandon_audio_focus() {
    let _ = system_worker().send(Ask::AbandonFocus);
}

/// The player's state to the media session (its notification, the lock
/// screen) and the playback service: on the system's thread.
pub fn playback(state: &Playback) {
    let _ = system_worker().send(Ask::Playback(state.to_line()));
}

/// Back at the top of the app: to the home screen (the activity is kept).
pub fn leave() {
    imp::call(CALL_LEAVE, "");
}

/// The screen stays on (while music plays under the picture) or may sleep
/// (on the system's thread, see [`system_worker`]).
pub fn keep_screen_on(on: bool) {
    let _ = system_worker().send(Ask::ScreenOn(on));
}

/// A line for the device's log (logcat, tag `axrossamp`) or the terminal.
pub fn log_line(text: &str) {
    imp::log_line(text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // (one test: the hub is the process's)
    #[test]
    fn events_are_queued_and_interrupts_are_told_at_once() {
        let _one_at_a_time = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_player_hook();
        let woken = Arc::new(AtomicUsize::new(0));
        let told = Arc::new(Mutex::new(Vec::new()));
        let (w, t) = (woken.clone(), told.clone());
        set_waker(move || {
            w.fetch_add(1, Ordering::Relaxed);
        });
        set_interrupt_hook(move |what| t.lock().unwrap().push(what));
        poll();

        dispatch(EVENT_INSETS, 0, 96, -4, 48, "");
        dispatch(
            EVENT_PICKED,
            2,
            1,
            7,
            2,
            "content://x/1\tA\tone.mp3\ncontent://x/2\tA\ttwo.mp3\n",
        );
        dispatch(EVENT_PICKED, -1, 0, 0, 0, "");
        dispatch(EVENT_LIFECYCLE, 1, 0, 0, 0, "");
        dispatch(EVENT_LIFECYCLE, 0, 0, 0, 0, "");
        dispatch(EVENT_AUDIO_FOCUS, FOCUS_LOSS_TRANSIENT, 0, 0, 0, "");
        dispatch(EVENT_AUDIO_FOCUS, FOCUS_GAIN, 0, 0, 0, "");
        dispatch(EVENT_AUDIO_FOCUS, FOCUS_LOSS, 0, 0, 0, "");
        // (ducking and unknown values: nothing to do)
        dispatch(
            EVENT_AUDIO_FOCUS,
            FOCUS_LOSS_TRANSIENT_CAN_DUCK,
            0,
            0,
            0,
            "",
        );
        dispatch(EVENT_AUDIO_FOCUS, 77, 0, 0, 0, "");
        dispatch(EVENT_NOISY, 0, 0, 0, 0, "");
        // (for the player, at once: before it is there, kept for it)
        dispatch(EVENT_MEDIA, MEDIA_NEXT, 0, 0, 0, "");
        dispatch(EVENT_MEDIA, MEDIA_SEEK_TO, 12_000, 0, 0, "");
        // (logged at once, nothing queued)
        dispatch(EVENT_STALL, 2500, 0, 0, 0, "");
        dispatch(999, 0, 0, 0, 0, "");

        // (told at once, before any frame)
        assert_eq!(
            *told.lock().unwrap(),
            [
                Interrupt::FocusLostForNow,
                Interrupt::FocusBack,
                Interrupt::FocusLost,
                Interrupt::Noisy,
            ]
        );
        let events = poll();
        assert_eq!(woken.load(Ordering::Relaxed), events.len());
        assert_eq!(events[0], Event::Insets([0, 96, 0, 48]));
        match &events[1] {
            Event::Picked {
                tracks,
                pick,
                more,
                total,
            } => {
                assert_eq!(tracks.len(), 2);
                assert_eq!(tracks[1].name, "two.mp3");
                assert_eq!((*pick, *more, *total), (7, true, 2));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(events[2], Event::NoPicker);
        assert_eq!(events[3], Event::Shown(true));
        assert_eq!(events[4], Event::Shown(false));
        assert_eq!(events.len(), 9);
        assert!(poll().is_empty());
        // (the media commands went to the player, kept until it is there)
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        set_player_hook(move |what| g.lock().unwrap().push(what));
        dispatch(EVENT_MEDIA, MEDIA_PAUSE, 0, 0, 0, "");
        assert_eq!(
            *got.lock().unwrap(),
            [
                ToPlayer::Media(MEDIA_NEXT, 0),
                ToPlayer::Media(MEDIA_SEEK_TO, 12_000),
                ToPlayer::Media(MEDIA_PAUSE, 0),
            ]
        );
        assert_eq!(
            Playback {
                state: 3,
                position_ms: 1500,
                duration_ms: 200_000,
                title: "A\ttitle".into(),
                artist: "Someone".into(),
                album: String::new(),
            }
            .to_line(),
            "3\t1500\t200000\tA title\tSomeone\t"
        );
    }
}
