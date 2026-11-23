//! axross-amp on Android.
//!
//! The player is the desktop program's library crate ([`amp`]: the engine,
//! the analysis, the 69 faces and their flash guard), used as it is; this
//! crate adds only what a phone needs around it:
//!
//! - [`screen`]: the one touch screen (an `eframe::App`);
//! - [`player`]: the engine and the playlist driven for picked tracks -
//!   nothing in it knows of a window (step 2's playback service drives the
//!   same thing);
//! - [`source`], [`opener`]: content:// documents made openable for the
//!   engine, off the window's thread;
//! - [`transport`]: the engine's commands from several threads, so that a
//!   pause holds;
//! - [`platform`]: the seam to Android (`Bridge.java`) and its stand-in on
//!   the host.
//!
//! On Android the activity (a `NativeActivity`) loads this library and
//! calls [`android_main`]; on the host `src/bin/preview.rs` opens the same
//! screen in a desktop window.

pub use axross_amp as amp;

pub mod gesture;
pub mod opener;
pub mod picked;
pub mod platform;
pub mod player;
pub mod screen;
pub mod source;
pub mod transport;

/// The activity's entry point (the android-activity glue calls it on a
/// thread of its own once the activity is created).
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew};
    use eframe::wgpu;

    platform::android::init_logging();
    platform::android::init(&app);
    platform::log_line(concat!(
        "axross-amp-android ",
        env!("CARGO_PKG_VERSION"),
        " starting"
    ));

    // Vulkan where the device has it, OpenGL ES otherwise (wgpu tries them
    // in that order). `adb shell setprop debug.axrossamp.backend gl` (or
    // `vulkan`) forces one - for a device whose Vulkan driver misbehaves,
    // and for the emulator in CI.
    let backends = match platform::android::system_property("debug.axrossamp.backend").as_str() {
        "gl" | "gles" => wgpu::Backends::GL,
        "vulkan" => wgpu::Backends::VULKAN,
        _ => wgpu::Backends::VULKAN | wgpu::Backends::GL,
    };
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = backends;
    // (what a phone's GPU offers, not a desktop's defaults: egui and the
    // faces' shaders need no more than the downlevel limits - the desktop's
    // GL path runs on them - and no texture larger than the adapter has)
    setup.device_descriptor = std::sync::Arc::new(|adapter| {
        let base = if adapter.get_info().backend == wgpu::Backend::Gl {
            wgpu::Limits::downlevel_webgl2_defaults()
        } else {
            wgpu::Limits::downlevel_defaults()
        };
        wgpu::DeviceDescriptor {
            label: Some("axross-amp device"),
            required_limits: base.using_resolution(adapter.limits()),
            ..Default::default()
        }
    });

    let mut options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        android_app: Some(app.clone()),
        ..Default::default()
    };
    options.wgpu_options.wgpu_setup = WgpuSetup::CreateNew(setup);

    let cache = app.internal_data_path().map(|dir| dir.join("open"));
    let result = eframe::run_native(
        "axross-amp",
        options,
        Box::new(move |cc| {
            Ok(Box::new(screen::Screen::new(
                cc,
                screen::Config {
                    cache,
                    tracks: Vec::new(),
                },
            )))
        }),
    );
    if let Err(e) = result {
        log::error!(target: "axrossamp", "the window ended with an error: {e}");
    }
    // The activity is gone (Back, or the system closed it). winit cannot
    // build a second event loop in one process, and a start of the activity
    // in this same process would call android_main again: leave, so that
    // the next start is a fresh process.
    platform::log_line("the activity ended; leaving the process");
    std::process::exit(0);
}
