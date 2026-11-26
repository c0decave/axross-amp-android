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

pub mod backend;
pub mod gesture;
pub mod opener;
pub mod picked;
pub mod platform;
pub mod player;
pub mod rest;
pub mod screen;
pub mod source;
pub mod transport;
pub mod watch;

#[cfg(all(test, target_os = "linux"))]
mod stall_tests;

#[cfg(test)]
mod faces_tests;

/// The activity's entry point (the android-activity glue calls it on a
/// thread of its own once the activity is created).
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew};
    use eframe::wgpu;

    platform::android::init_logging();
    platform::android::init(&app);
    // (file names in the log: a debug build's, the debug APK's - which is
    // built with the release's settings, see Cargo.toml's preview profile)
    watch::set_names(cfg!(debug_assertions) || platform::android::package().ends_with(".debug"));
    watch::start();
    platform::log_line(concat!(
        "axross-amp-android ",
        env!("CARGO_PKG_VERSION"),
        " starting"
    ));

    // One backend, chosen before the instance is made (src/backend.rs):
    // Vulkan on a real GPU from Android 10 on, OpenGL ES elsewhere, on the
    // emulator and where Vulkan failed before. `adb shell setprop
    // debug.axrossamp.backend gl` (or `vulkan`) forces one.
    let data = app.internal_data_path();
    let choice = choose_backend(data.as_deref().is_some_and(backend::gl_marked));
    let vulkan = choice.backend == backend::Backend::Vulkan;
    let backends = if vulkan {
        wgpu::Backends::VULKAN
    } else {
        wgpu::Backends::GL
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

    // (documents being played get their names in the cache: what is copied
    // there - a provider that cannot open in place - the system may
    // reclaim when storage runs short)
    let cache = data
        .as_deref()
        .and_then(std::path::Path::parent)
        .map(|app_dir| app_dir.join("cache").join("open"));
    let config = screen::Config {
        cache,
        tracks: Vec::new(),
        data: data.clone(),
        vulkan,
    };
    // (a panic on this thread - with unwinding, see Cargo.toml - ends the
    // window, and the process below: never music playing on without one)
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eframe::run_native(
            "axross-amp",
            options,
            Box::new(move |cc| Ok(Box::new(screen::Screen::new(cc, config)))),
        )
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            log::error!(target: "axrossamp", "the window ended with an error: {e}");
            // (Vulkan that does not come up here: OpenGL ES next time)
            if vulkan && let Some(dir) = data.as_deref() {
                backend::mark_gl(dir, &format!("the window ended with an error: {e}"));
            }
        }
        Err(_) => log::error!(target: "axrossamp", "the window's thread panicked"),
    }
    // The activity is gone (Back, or the system closed it). winit cannot
    // build a second event loop in one process, and a start of the activity
    // in this same process would call android_main again: leave, so that
    // the next start is a fresh process.
    platform::log_line("the activity ended; leaving the process");
    std::process::exit(0);
}

/// The system's answers and, where needed, Vulkan's adapters (in an
/// instance of their own, without a surface), for [`backend::System::choose`].
/// `gl_marked`: Vulkan failed on this device before.
#[cfg(target_os = "android")]
fn choose_backend(gl_marked: bool) -> backend::Choice {
    use eframe::wgpu;
    use platform::android::system_property;

    let hardware = system_property("ro.hardware");
    let system = backend::System {
        forced: system_property("debug.axrossamp.backend"),
        api_level: system_property("ro.build.version.sdk")
            .trim()
            .parse()
            .unwrap_or(0),
        emulator: system_property("ro.kernel.qemu") == "1"
            || system_property("ro.boot.qemu") == "1"
            || matches!(hardware.as_str(), "ranchu" | "goldfish"),
        gl_marked,
    };
    let vulkan = if system.needs_probe() {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(descriptor);
        backend::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .iter()
            .map(|adapter| {
                let info = adapter.get_info();
                backend::VulkanAdapter {
                    name: info.name,
                    cpu: info.device_type == wgpu::DeviceType::Cpu,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let choice = system.choose(&vulkan);
    platform::log_line(&format!(
        "backend: {:?} ({}; API {}{})",
        choice.backend,
        choice.why,
        system.api_level,
        if system.emulator { ", emulator" } else { "" },
    ));
    choice
}
