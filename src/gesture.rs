//! What a finger did, as pure functions of time and place (the screen feeds
//! them egui's input; the tests feed them numbers):
//!
//! - a **swipe** to the side over the picture steps through the faces;
//! - a **tap** on the picture shows or hides the controls;
//! - a **double tap** on the picture's left or right third skips 10 s back
//!   or on ([`Taps`]); more taps on the same side, each within the window,
//!   skip on again (as video players do);
//! - a **long press** on previous or next winds back or on while it is
//!   held, faster the longer ([`wind_speed`]).
//!
//! How they keep apart: a swipe is a drag (egui tells a drag from a tap by
//! the finger's travel), so it never counts as a tap. A tap in the middle
//! third acts at once; a tap on a side third waits [`DOUBLE_TAP`] seconds
//! for a second one before it shows or hides the controls - the price of
//! the double tap is that delay, and only on the sides.

/// The step a horizontal swipe asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Next,
    Previous,
}

/// The least a finger travels for a swipe, in points (dp): the larger of a
/// touch target's width and an eighth of the picture's.
pub fn least_travel(width: f32) -> f32 {
    (width / 8.0).max(48.0)
}

/// A drag of (`dx`, `dy`) points over a picture `width` points wide: a
/// swipe to the left is the next face, to the right the one before. Too
/// short, or more up or down than sideways (within about 34 degrees of the
/// horizontal counts), is none.
pub fn swipe(dx: f32, dy: f32, width: f32) -> Option<Step> {
    if !dx.is_finite() || !dy.is_finite() || !width.is_finite() {
        return None;
    }
    if dx.abs() < least_travel(width) || dx.abs() < 1.5 * dy.abs() {
        return None;
    }
    Some(if dx < 0.0 { Step::Next } else { Step::Previous })
}

// -- taps --------------------------------------------------------------------------------------

/// Seconds between the first tap and the second of a double tap (and
/// between the taps of a chain of skips).
pub const DOUBLE_TAP: f64 = 0.3;

/// The farthest apart two taps of a double tap may be, in points.
pub const DOUBLE_TAP_REACH: f32 = 80.0;

/// Seconds a double tap skips.
pub const DOUBLE_TAP_SKIP: f64 = 10.0;

/// What a tap on the picture does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TapAction {
    /// Show or hide the controls.
    Controls,
    /// Skip that many seconds (negative: back).
    Skip(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Zone {
    Left,
    Middle,
    Right,
}

fn zone(x: f32, width: f32) -> Zone {
    let third = width / 3.0;
    if !x.is_finite() || !width.is_finite() || width <= 0.0 {
        Zone::Middle
    } else if x < third {
        Zone::Left
    } else if x > width - third {
        Zone::Right
    } else {
        Zone::Middle
    }
}

#[derive(Clone, Copy, Debug)]
struct Tap {
    at: f64,
    x: f32,
    y: f32,
    zone: Zone,
}

impl Tap {
    fn pairs_with(&self, other: &Tap) -> bool {
        self.zone == other.zone
            && other.at - self.at <= DOUBLE_TAP
            && other.at >= self.at
            && (other.x - self.x).hypot(other.y - self.y) <= DOUBLE_TAP_REACH
    }
}

/// Tells single taps from double taps (see the module).
#[derive(Clone, Debug, Default)]
pub struct Taps {
    /// A side tap waiting for its second.
    pending: Option<Tap>,
    /// The last tap of a double tap (or of the skips after it): another on
    /// the same side within the window skips again.
    chain: Option<Tap>,
}

impl Taps {
    /// A tap at (`x`, `y`) on a picture `width` points wide at `now`
    /// seconds: what it does at once (a side tap may still be waiting, see
    /// [`Taps::tick`]).
    pub fn tap(&mut self, now: f64, x: f32, y: f32, width: f32) -> Option<TapAction> {
        let tap = Tap {
            at: now,
            x,
            y,
            zone: zone(x, width),
        };
        let skip = |zone| {
            TapAction::Skip(if zone == Zone::Left {
                -DOUBLE_TAP_SKIP
            } else {
                DOUBLE_TAP_SKIP
            })
        };
        if let Some(last) = self.chain.take()
            && last.pairs_with(&tap)
        {
            self.chain = Some(tap);
            return Some(skip(tap.zone));
        }
        let waiting = self.pending.take();
        if let Some(first) = waiting
            && first.pairs_with(&tap)
        {
            self.chain = Some(tap);
            return Some(skip(tap.zone));
        }
        // (a side tap that waited and got no second of its own: a single tap
        // after all, decided now)
        let earlier = waiting.map(|_| TapAction::Controls);
        if tap.zone == Zone::Middle {
            // (two single taps at once: they cancel out)
            return if earlier.is_some() {
                None
            } else {
                Some(TapAction::Controls)
            };
        }
        self.pending = Some(tap);
        earlier
    }

    /// Time going by: a side tap whose second did not come in time is a
    /// single tap.
    pub fn tick(&mut self, now: f64) -> Option<TapAction> {
        if let Some(chain) = self.chain
            && now - chain.at > DOUBLE_TAP
        {
            self.chain = None;
        }
        match self.pending {
            Some(first) if now - first.at > DOUBLE_TAP => {
                self.pending = None;
                Some(TapAction::Controls)
            }
            _ => None,
        }
    }

    /// Whether a tap is still waiting (the screen keeps drawing frames to
    /// decide it).
    pub fn waiting(&self) -> bool {
        self.pending.is_some() || self.chain.is_some()
    }
}

// -- long presses ------------------------------------------------------------------------------

/// Seconds a finger stays on previous or next before it winds.
pub const LONG_PRESS: f64 = 0.5;

/// Seconds of the track wound for each second held, `held` seconds after
/// the finger went down: nothing before [`LONG_PRESS`], then faster in
/// steps - 8 times, 24 times after 2.5 s, 60 times after 5 s.
pub fn wind_speed(held: f64) -> f64 {
    if !held.is_finite() || held < LONG_PRESS {
        0.0
    } else if held < 2.5 {
        8.0
    } else if held < 5.0 {
        24.0
    } else {
        60.0
    }
}

/// How far a press held from 0 to `held` seconds has wound, in seconds of
/// the track (the integral of [`wind_speed`]).
pub fn wound(held: f64) -> f64 {
    if !held.is_finite() || held <= LONG_PRESS {
        return 0.0;
    }
    let part = |from: f64, to: f64, speed: f64| (held.min(to) - from).max(0.0) * speed;
    part(LONG_PRESS, 2.5, 8.0) + part(2.5, 5.0, 24.0) + part(5.0, f64::INFINITY, 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_swipe_to_the_left_is_the_next_face() {
        assert_eq!(swipe(-120.0, 10.0, 400.0), Some(Step::Next));
        assert_eq!(swipe(120.0, -10.0, 400.0), Some(Step::Previous));
    }

    #[test]
    fn a_short_or_steep_drag_is_no_swipe() {
        // (shorter than a touch target)
        assert_eq!(swipe(-40.0, 0.0, 300.0), None);
        // (an eighth of a wide picture is more than 48)
        assert_eq!(swipe(-90.0, 0.0, 800.0), None);
        assert_eq!(swipe(-101.0, 0.0, 800.0), Some(Step::Next));
        // (mostly down: a scroll, not a swipe)
        assert_eq!(swipe(-100.0, 90.0, 400.0), None);
        assert_eq!(swipe(f32::NAN, 0.0, 400.0), None);
    }

    const W: f32 = 390.0;

    #[test]
    fn a_tap_in_the_middle_shows_the_controls_at_once() {
        let mut taps = Taps::default();
        assert_eq!(taps.tap(1.0, 195.0, 400.0, W), Some(TapAction::Controls));
        assert!(!taps.waiting());
        assert_eq!(taps.tick(2.0), None);
    }

    #[test]
    fn a_single_tap_on_a_side_waits_for_a_second_then_shows_the_controls() {
        let mut taps = Taps::default();
        assert_eq!(taps.tap(1.0, 40.0, 400.0, W), None);
        assert!(taps.waiting());
        assert_eq!(taps.tick(1.2), None);
        assert_eq!(taps.tick(1.31), Some(TapAction::Controls));
        assert_eq!(taps.tick(1.5), None);
    }

    #[test]
    fn a_double_tap_on_a_side_skips_and_more_taps_skip_on() {
        let mut taps = Taps::default();
        assert_eq!(taps.tap(1.0, 350.0, 400.0, W), None);
        assert_eq!(taps.tap(1.2, 355.0, 410.0, W), Some(TapAction::Skip(10.0)));
        // (a third and a fourth, each within the window: on again)
        assert_eq!(taps.tap(1.45, 352.0, 405.0, W), Some(TapAction::Skip(10.0)));
        assert_eq!(taps.tap(1.7, 350.0, 400.0, W), Some(TapAction::Skip(10.0)));
        // (no single tap is left over to show the controls)
        assert_eq!(taps.tick(3.0), None);
        assert!(!taps.waiting());
        let mut taps = Taps::default();
        assert_eq!(taps.tap(5.0, 20.0, 300.0, W), None);
        assert_eq!(taps.tap(5.25, 30.0, 310.0, W), Some(TapAction::Skip(-10.0)));
    }

    #[test]
    fn taps_too_slow_too_far_or_on_two_sides_are_single_taps() {
        // (too slow: two single taps, the controls shown and hidden)
        let mut taps = Taps::default();
        assert_eq!(taps.tap(1.0, 40.0, 400.0, W), None);
        assert_eq!(taps.tick(1.35), Some(TapAction::Controls));
        assert_eq!(taps.tap(1.4, 40.0, 400.0, W), None);
        assert_eq!(taps.tick(1.75), Some(TapAction::Controls));
        // (too far apart on one side)
        let mut taps = Taps::default();
        assert_eq!(taps.tap(1.0, 40.0, 100.0, W), None);
        assert_eq!(taps.tap(1.1, 40.0, 500.0, W), Some(TapAction::Controls));
        assert_eq!(taps.tick(1.5), Some(TapAction::Controls));
        // (left, then right: no skip)
        let mut taps = Taps::default();
        assert_eq!(taps.tap(1.0, 40.0, 400.0, W), None);
        assert_eq!(taps.tap(1.1, 350.0, 400.0, W), Some(TapAction::Controls));
        assert_eq!(taps.tick(1.5), Some(TapAction::Controls));
        // (a side tap, then one in the middle: two single taps, cancelled)
        let mut taps = Taps::default();
        assert_eq!(taps.tap(1.0, 40.0, 400.0, W), None);
        assert_eq!(taps.tap(1.1, 195.0, 400.0, W), None);
        assert!(!taps.waiting());
    }

    #[test]
    fn a_chain_ends_with_its_window() {
        let mut taps = Taps::default();
        taps.tap(1.0, 350.0, 400.0, W);
        assert_eq!(taps.tap(1.2, 350.0, 400.0, W), Some(TapAction::Skip(10.0)));
        assert_eq!(taps.tick(1.6), None);
        // (a tap after the chain's window is a first tap again)
        assert_eq!(taps.tap(1.7, 350.0, 400.0, W), None);
        assert_eq!(taps.tick(2.1), Some(TapAction::Controls));
    }

    #[test]
    fn odd_numbers_are_the_middle() {
        let mut taps = Taps::default();
        assert_eq!(taps.tap(1.0, f32::NAN, 0.0, W), Some(TapAction::Controls));
        assert_eq!(taps.tap(2.0, 10.0, 0.0, 0.0), Some(TapAction::Controls));
    }

    #[test]
    fn a_long_press_winds_faster_the_longer_it_is_held() {
        assert_eq!(wind_speed(0.0), 0.0);
        assert_eq!(wind_speed(0.49), 0.0);
        assert_eq!(wind_speed(0.5), 8.0);
        assert_eq!(wind_speed(3.0), 24.0);
        assert_eq!(wind_speed(9.0), 60.0);
        assert_eq!(wind_speed(f64::NAN), 0.0);
        assert_eq!(wound(0.4), 0.0);
        assert!((wound(1.5) - 8.0).abs() < 1e-9);
        assert!((wound(2.5) - 16.0).abs() < 1e-9);
        assert!((wound(5.0) - (16.0 + 60.0)).abs() < 1e-9);
        assert!((wound(6.0) - (16.0 + 60.0 + 60.0)).abs() < 1e-9);
        // (it only grows)
        let mut last = 0.0;
        for i in 0..200 {
            let now = wound(i as f64 * 0.05);
            assert!(now >= last);
            last = now;
        }
    }
}
