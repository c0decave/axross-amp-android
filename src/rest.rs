//! The app at rest while it is hidden: nothing of it keeps the phone awake.
//!
//! Hidden (the home screen, another app, the screen locked: no playback
//! service yet, so the music is paused), it waits [`REST_AFTER`] (a quick
//! look at a notification or a picker shown and closed again costs
//! nothing), and then rests: the engine ends (its output stream is closed;
//! an open AAudio stream keeps the audio server and its wakelock busy even
//! when it plays silence), the meters stop, the watchdog only looks now and
//! then.
//! Shown again, the screen starts what rested anew ([`Rest::shown`]): a new
//! engine, the track where it was, paused.
//!
//! What rests is told by the callback given at the start; it runs on the
//! thread that waited, under the lock that [`Rest::shown`] takes too, so a
//! rest that has begun is over before anything starts anew.

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long the app is hidden before it rests.
pub const REST_AFTER: Duration = Duration::from_secs(5);

struct State {
    /// Counts hide and show: a wait that is no longer the latest does not
    /// rest.
    round: u64,
    resting: bool,
}

pub struct Rest {
    state: Mutex<State>,
    after: Duration,
    on_rest: Box<dyn Fn() + Send + Sync>,
}

impl Rest {
    pub fn new(after: Duration, on_rest: impl Fn() + Send + Sync + 'static) -> Arc<Rest> {
        Arc::new(Rest {
            state: Mutex::new(State {
                round: 0,
                resting: false,
            }),
            after,
            on_rest: Box::new(on_rest),
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The app is hidden (any thread): it rests after the wait, unless it
    /// is shown again before.
    pub fn hidden(self: &Arc<Self>) {
        let round = {
            let mut state = self.state();
            state.round += 1;
            state.round
        };
        let me = self.clone();
        let _ = std::thread::Builder::new()
            .name("amp-rest".into())
            .spawn(move || {
                std::thread::sleep(me.after);
                let mut state = me.state();
                if state.round == round && !state.resting {
                    state.resting = true;
                    (me.on_rest)();
                    crate::platform::log_line("hidden: at rest (no engine, no meters)");
                }
            });
    }

    /// The app is shown again: true if it rested (what rested is to be
    /// started anew - the rest is over when this returns).
    pub fn shown(&self) -> bool {
        let mut state = self.state();
        state.round += 1;
        std::mem::take(&mut state.resting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn counting(after: Duration) -> (Arc<Rest>, Arc<AtomicUsize>) {
        let rested = Arc::new(AtomicUsize::new(0));
        let count = rested.clone();
        let rest = Rest::new(after, move || {
            count.fetch_add(1, Ordering::Relaxed);
        });
        (rest, rested)
    }

    #[test]
    fn hidden_long_enough_it_rests_once() {
        let (rest, rested) = counting(Duration::from_millis(30));
        rest.hidden();
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(rested.load(Ordering::Relaxed), 1);
        // (hidden again while resting: no second rest)
        rest.hidden();
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(rested.load(Ordering::Relaxed), 1);
        assert!(rest.shown());
        assert!(!rest.shown());
    }

    #[test]
    fn shown_again_in_time_it_does_not_rest() {
        let (rest, rested) = counting(Duration::from_millis(80));
        rest.hidden();
        std::thread::sleep(Duration::from_millis(20));
        assert!(!rest.shown());
        // (hidden and shown quickly several times: only the latest counts)
        rest.hidden();
        assert!(!rest.shown());
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(rested.load(Ordering::Relaxed), 0);
    }
}
