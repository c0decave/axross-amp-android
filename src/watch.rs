//! What the app's threads are doing, for the device's log when one of them
//! waits too long - the next report from a phone says where it waited.
//!
//! Each thread names its step ([`doing`]) and its end ([`done`]); a
//! watchdog ([`start`]) logs a step still under way after [`SLOW`], and how
//! long it took once it ends. The Java side's watchdog (the main thread
//! blocked) asks for all of them at once ([`report`]).
//!
//! File names are only logged by debug builds ([`name`]).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A step taking longer than this is logged.
pub const SLOW: Duration = Duration::from_secs(2);

struct Slot {
    thread: &'static str,
    what: String,
    since: Instant,
    told: bool,
}

static SLOTS: Mutex<Vec<Slot>> = Mutex::new(Vec::new());

fn slots() -> std::sync::MutexGuard<'static, Vec<Slot>> {
    SLOTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `thread` is doing `what` from now on.
pub fn doing(thread: &'static str, what: impl Into<String>) {
    let mut slots = slots();
    let slot = Slot {
        thread,
        what: what.into(),
        since: Instant::now(),
        told: false,
    };
    match slots.iter_mut().find(|s| s.thread == thread) {
        Some(old) => *old = slot,
        None => slots.push(slot),
    }
}

/// `thread` is done with its step: how long it took.
pub fn done(thread: &'static str) -> Duration {
    let mut slots = slots();
    let Some(at) = slots.iter().position(|s| s.thread == thread) else {
        return Duration::ZERO;
    };
    let slot = slots.remove(at);
    let took = slot.since.elapsed();
    if slot.told {
        crate::platform::log_line(&format!(
            "{}: {} took {:.1} s",
            slot.thread,
            slot.what,
            took.as_secs_f32()
        ));
    }
    took
}

/// The steps under way, one line each ("" when none is).
pub fn report() -> String {
    let slots = slots();
    slots
        .iter()
        .map(|s| {
            format!(
                "{}: {} for {:.1} s",
                s.thread,
                s.what,
                s.since.elapsed().as_secs_f32()
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The steps under way longer than `slow` that were not logged yet: logged
/// now (the watchdog's round; the tests call it themselves).
pub fn check(slow: Duration) -> Vec<String> {
    let mut slots = slots();
    let mut lines = Vec::new();
    for slot in slots.iter_mut().filter(|s| !s.told) {
        let for_ = slot.since.elapsed();
        if for_ >= slow {
            slot.told = true;
            lines.push(format!(
                "{}: still {} after {:.1} s",
                slot.thread,
                slot.what,
                for_.as_secs_f32()
            ));
        }
    }
    drop(slots);
    for line in &lines {
        crate::platform::log_line(line);
    }
    lines
}

/// The watchdog: twice a second, the steps that take too long.
pub fn start() {
    let _ = std::thread::Builder::new()
        .name("amp-watch".into())
        .spawn(|| {
            loop {
                let quiet = QUIET.load(Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(if quiet { 10_000 } else { 500 }));
                check(SLOW);
            }
        });
}

static NAMES: AtomicBool = AtomicBool::new(cfg!(debug_assertions));

/// Whether file names go to the log (a debug build, the debug APK).
pub fn set_names(on: bool) {
    NAMES.store(on, Ordering::Relaxed);
}

/// Quiet: the app is hidden - the watchdog looks only now and then, and
/// wakes the device no more than that.
static QUIET: AtomicBool = AtomicBool::new(false);

pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

/// A file's name for the log: itself in a debug build, only its ending in
/// a release (no names of the user's files in a phone's log).
pub fn name(name: &str) -> String {
    if NAMES.load(Ordering::Relaxed) {
        name.to_owned()
    } else {
        match crate::picked::extension(name) {
            Some(ending) => format!("a .{ending} file"),
            None => "a file".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_that_takes_too_long_is_told_once_and_its_end_too() {
        doing("test-slow", "opening x");
        assert!(report().contains("test-slow: opening x for"));
        assert!(
            check(Duration::from_secs(3600))
                .iter()
                .all(|l| !l.contains("test-slow"))
        );
        std::thread::sleep(Duration::from_millis(30));
        let told = check(Duration::from_millis(10));
        assert!(
            told.iter()
                .any(|l| l.starts_with("test-slow: still opening x after"))
        );
        // (once)
        assert!(
            check(Duration::from_millis(10))
                .iter()
                .all(|l| !l.contains("test-slow"))
        );
        assert!(done("test-slow") >= Duration::from_millis(30));
        assert!(!report().contains("test-slow"));
        assert_eq!(done("test-slow"), Duration::ZERO);
    }

    #[test]
    fn names_are_kept_out_of_a_release_log() {
        set_names(false);
        assert_eq!(name("01 One.flac"), "a .flac file");
        assert_eq!(name("track"), "a file");
        set_names(true);
        assert_eq!(name("01 One.flac"), "01 One.flac");
    }
}
