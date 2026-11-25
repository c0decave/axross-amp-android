//! Which of wgpu's backends draws: Vulkan or OpenGL ES - one of them, never
//! both in one instance.
//!
//! Both in one instance does not work on Android: creating the window's
//! surface creates it for every backend of the instance, and once the
//! Vulkan surface has connected to the native window, EGL can no longer
//! (`eglCreateWindowSurface`: "already connected to another API",
//! `EGL_BAD_ALLOC`) - so the GL adapter wgpu falls back to gets no surface.
//! The choice is therefore made before the instance is, from what the
//! system says and from a look at Vulkan's adapters in an instance of their
//! own.
//!
//! Vulkan is taken where it is likely to work: a real GPU's driver, Android
//! 10 (API 29) or later - when Vulkan 1.1 became a requirement for 64-bit
//! devices and the drivers grew up. OpenGL ES 3 everywhere else: older
//! Android versions, a Vulkan that is only a software rasterizer, and the
//! emulator, whose Vulkan (SwiftShader behind the emulator's translation)
//! rejects wgpu's shaders and stalls. `adb shell setprop
//! debug.axrossamp.backend gl|vulkan|auto` overrides the choice.

/// The oldest Android version Vulkan is chosen on by itself.
pub const VULKAN_MIN_API: u32 = 29;

/// What the choice is made from.
#[derive(Clone, Debug, Default)]
pub struct System {
    /// `debug.axrossamp.backend`: "gl", "gles", "vulkan", or anything else
    /// (unset, "auto") for the app's own choice.
    pub forced: String,
    /// `ro.build.version.sdk`.
    pub api_level: u32,
    /// An emulator (`ro.kernel.qemu`, `ro.boot.qemu`, `ro.hardware`).
    pub emulator: bool,
}

/// One Vulkan adapter, as the probe saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VulkanAdapter {
    pub name: String,
    /// A software rasterizer (wgpu's `DeviceType::Cpu`).
    pub cpu: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Vulkan,
    Gl,
}

/// The choice and why, for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub backend: Backend,
    pub why: String,
}

impl System {
    /// Whether Vulkan's adapters have to be looked at: only for the app's
    /// own choice, and only where Vulkan could be chosen at all.
    pub fn needs_probe(&self) -> bool {
        self.forced_backend().is_none() && !self.emulator && self.api_level >= VULKAN_MIN_API
    }

    fn forced_backend(&self) -> Option<Backend> {
        match self.forced.trim().to_ascii_lowercase().as_str() {
            "gl" | "gles" | "opengl" => Some(Backend::Gl),
            "vulkan" | "vk" => Some(Backend::Vulkan),
            _ => None,
        }
    }

    /// The backend, given the Vulkan adapters the probe found (none when it
    /// was not needed or Vulkan is not there).
    pub fn choose(&self, vulkan: &[VulkanAdapter]) -> Choice {
        let choice = |backend, why: &str| Choice {
            backend,
            why: why.to_owned(),
        };
        if let Some(backend) = self.forced_backend() {
            return choice(backend, "forced by debug.axrossamp.backend");
        }
        if self.emulator {
            return choice(Backend::Gl, "an emulator: its Vulkan is not a GPU's");
        }
        if self.api_level < VULKAN_MIN_API {
            return choice(Backend::Gl, "Android before 10: OpenGL ES");
        }
        match vulkan.iter().find(|adapter| !adapter.cpu) {
            Some(adapter) => Choice {
                backend: Backend::Vulkan,
                why: format!("Vulkan on {}", adapter.name),
            },
            None if vulkan.is_empty() => choice(Backend::Gl, "no Vulkan adapter"),
            None => choice(Backend::Gl, "Vulkan only in software"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone(api_level: u32) -> System {
        System {
            forced: String::new(),
            api_level,
            emulator: false,
        }
    }

    fn gpu() -> Vec<VulkanAdapter> {
        vec![VulkanAdapter {
            name: "Mali-G78".into(),
            cpu: false,
        }]
    }

    #[test]
    fn a_recent_phone_with_a_gpu_gets_vulkan() {
        let system = phone(34);
        assert!(system.needs_probe());
        assert_eq!(system.choose(&gpu()).backend, Backend::Vulkan);
    }

    #[test]
    fn the_emulator_gets_gl_without_a_probe() {
        let system = System {
            emulator: true,
            ..phone(34)
        };
        assert!(!system.needs_probe());
        assert_eq!(system.choose(&gpu()).backend, Backend::Gl);
    }

    #[test]
    fn old_android_gets_gl() {
        let system = phone(26);
        assert!(!system.needs_probe());
        assert_eq!(system.choose(&[]).backend, Backend::Gl);
    }

    #[test]
    fn software_or_no_vulkan_gets_gl() {
        let system = phone(34);
        let software = vec![VulkanAdapter {
            name: "SwiftShader Device".into(),
            cpu: true,
        }];
        assert_eq!(system.choose(&software).backend, Backend::Gl);
        assert_eq!(system.choose(&[]).backend, Backend::Gl);
    }

    #[test]
    fn the_property_wins() {
        let emulator = System {
            forced: "vulkan".into(),
            emulator: true,
            ..phone(26)
        };
        assert!(!emulator.needs_probe());
        assert_eq!(emulator.choose(&[]).backend, Backend::Vulkan);
        let gl = System {
            forced: " GL ".into(),
            ..phone(34)
        };
        assert_eq!(gl.choose(&gpu()).backend, Backend::Gl);
        let auto = System {
            forced: "auto".into(),
            ..phone(34)
        };
        assert!(auto.needs_probe());
    }
}
