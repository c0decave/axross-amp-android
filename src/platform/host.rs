//! The host's stand-ins (the desktop preview, the tests): no pickers, no
//! audio focus to ask for, a terminal for a log.

use super::{CALL_REQUEST_FOCUS, Event};

pub fn call(what: i32, _arg: &str) -> i32 {
    match what {
        // (granted: nobody to ask)
        CALL_REQUEST_FOCUS => 1,
        super::CALL_OPEN_FD => -1,
        super::CALL_PICK_FOLDER | super::CALL_PICK_FILES => {
            // (the preview takes its music from the command line)
            let mut hub = super::hub();
            hub.events.push(Event::NoPicker);
            if let Some(wake) = hub.waker.as_ref() {
                wake();
            }
            0
        }
        _ => 0,
    }
}

pub fn keep_screen_on(_on: bool) {}

pub fn log_line(text: &str) {
    eprintln!("axrossamp: {text}");
}
