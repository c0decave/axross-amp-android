//! The phone's screen in a desktop window: to work on its layout without a
//! device. `cargo run --bin preview -- track.flac another.mp3` - the tracks
//! come from the command line (no pickers on the host); the window starts
//! in a phone's portrait shape and can be resized to landscape.
//!
//! Not part of the APK (cargo-ndk builds the library only).

#[cfg(not(target_os = "android"))]
fn main() -> eframe::Result {
    use axrossamp::screen::{Config, Screen};
    use eframe::egui;

    let tracks: Vec<String> = std::env::args().skip(1).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("axross-amp-android preview")
            .with_inner_size([412.0, 892.0])
            .with_min_inner_size([320.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "axross-amp-android preview",
        options,
        Box::new(move |cc| {
            Ok(Box::new(Screen::new(
                cc,
                Config {
                    cache: None,
                    tracks,
                    data: None,
                    vulkan: false,
                },
            )))
        }),
    )
}

#[cfg(target_os = "android")]
fn main() {}
