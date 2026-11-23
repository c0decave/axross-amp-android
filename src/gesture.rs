//! What a finger did on the picture: a swipe to the side steps through the
//! faces. (A tap - the controls shown or hidden - is egui's click.)

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
}
