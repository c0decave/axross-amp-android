//! Android's end of the seam: JNI to the activity (`MainActivity.bridgeCall`,
//! `Bridge.nativeEvent`), the NDK for the window's flags and the log.

use android_activity::AndroidApp;
use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::jint;
use jni::{EnvUnowned, JavaVM, jni_sig, jni_str};
use std::ffi::{CString, c_char, c_int};
use std::sync::Mutex;

static APP: Mutex<Option<AndroidApp>> = Mutex::new(None);

fn app() -> Option<AndroidApp> {
    APP.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// The activity this process runs (once, first thing in `android_main`).
pub fn init(app: &AndroidApp) {
    *APP.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(app.clone());
}

/// `int MainActivity.bridgeCall(int what, String arg)`, from any thread;
/// -1 when it could not be called.
pub fn call(what: i32, arg: &str) -> i32 {
    let Some(app) = app() else {
        return -1;
    };
    // Safety: the VM pointer is the one the activity was created with, valid
    // for the process's life.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let result = vm.attach_current_thread(|env| -> jni::errors::Result<i32> {
        // Safety: a global reference to the activity, kept by the
        // android-activity glue for as long as the activity lives; it is
        // only borrowed here (a JObject does not delete it when dropped).
        let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
        let text = env.new_string(arg)?;
        env.call_method(
            &activity,
            jni_str!("bridgeCall"),
            jni_sig!("(ILjava/lang/String;)I"),
            &[JValue::Int(what), JValue::Object(&text)],
        )?
        .i()
    });
    match result {
        Ok(value) => value,
        Err(e) => {
            log_line(&format!("bridge call {what} failed: {e}"));
            -1
        }
    }
}

/// `static native void Bridge.nativeEvent(int, int, int, int, int, String)`.
// (the name and the arguments are the Java method's)
#[allow(non_snake_case, clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_c0decave_axrossamp_Bridge_nativeEvent<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    kind: jint,
    a: jint,
    b: jint,
    c: jint,
    d: jint,
    text: JString<'caller>,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let text = if text.is_null() {
            String::new()
        } else {
            text.try_to_string(env)?
        };
        super::dispatch(kind, a, b, c, d, &text);
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>()
}

/// The screen kept on or not: by the Java side, on its main thread.
///
/// Not by android-activity's `set_window_flags`: that holds the
/// NativeActivity glue's lock while `ANativeActivity_setWindowFlags` waits
/// on the main thread - and the main thread, in onPause, waits for that
/// lock. Both threads stopped for good, the moment music began to play
/// (the first track played on, the app hung: CI's smoke test caught it,
/// "amp-system: still setting the window's flags").
pub fn keep_screen_on(on: bool) {
    call(super::CALL_SCREEN_ON, if on { "1" } else { "0" });
}

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

const ANDROID_LOG_INFO: c_int = 4;
const ANDROID_LOG_WARN: c_int = 5;
const ANDROID_LOG_ERROR: c_int = 6;

fn write(priority: c_int, text: &str) {
    // (a NUL inside ends the line there)
    let text = CString::new(text.replace('\0', " ")).unwrap_or_default();
    // Safety: two NUL-terminated strings that live through the call.
    unsafe {
        __android_log_write(priority, c"axrossamp".as_ptr(), text.as_ptr());
    }
}

pub fn log_line(text: &str) {
    write(ANDROID_LOG_INFO, text);
}

/// The `log` crate's lines (winit, wgpu, eframe: warnings and errors; this
/// crate's: all) and a panic's message to logcat.
pub fn init_logging() {
    struct Logcat;
    impl log::Log for Logcat {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Warn
                || (metadata.level() <= log::Level::Info
                    && metadata.target().starts_with("axrossamp"))
        }
        fn log(&self, record: &log::Record<'_>) {
            if !self.enabled(record.metadata()) {
                return;
            }
            let priority = match record.level() {
                log::Level::Error => ANDROID_LOG_ERROR,
                log::Level::Warn => ANDROID_LOG_WARN,
                _ => ANDROID_LOG_INFO,
            };
            write(priority, &format!("{}: {}", record.target(), record.args()));
        }
        fn flush(&self) {}
    }
    static LOGCAT: Logcat = Logcat;
    if log::set_logger(&LOGCAT).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    std::panic::set_hook(Box::new(|info| {
        write(ANDROID_LOG_ERROR, &format!("panic: {info}"));
    }));
}

/// A system property (`adb shell setprop debug.axrossamp.backend gl`), ""
/// when it is not set.
pub fn system_property(name: &str) -> String {
    unsafe extern "C" {
        fn __system_property_get(name: *const c_char, value: *mut c_char) -> c_int;
    }
    let Ok(name) = CString::new(name) else {
        return String::new();
    };
    // (PROP_VALUE_MAX is 92)
    let mut value = [0u8; 128];
    // Safety: the buffer is larger than the longest value the call writes.
    let length = unsafe { __system_property_get(name.as_ptr(), value.as_mut_ptr().cast()) };
    let length = (length.max(0) as usize).min(value.len());
    String::from_utf8_lossy(&value[..length]).into_owned()
}

/// This process's name: the package's (`io.github.c0decave.axrossamp`, the
/// debug APK's with `.debug`); "" when it cannot be read.
pub fn package() -> String {
    std::fs::read("/proc/self/cmdline")
        .ok()
        .and_then(|bytes| {
            let first = bytes.split(|b| *b == 0).next()?.to_vec();
            String::from_utf8(first).ok()
        })
        .unwrap_or_default()
}
