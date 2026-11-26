//! The one screen: a face over the whole display, and over it - shown and
//! hidden by a tap - the track's title, a seek bar, previous / 15 s back /
//! play / 15 s on / next, the pickers and the list of faces. A swipe to the
//! side steps through the faces; a double tap on the picture's left or
//! right third skips 10 s; a long press on previous or next winds (see
//! [`crate::gesture`]).
//!
//! Everything a finger can hit is at least 48 points (dp) in both
//! directions, and nothing to hit or read lies under a system bar or a
//! display cutout (the insets come from the activity).
//!
//! The faces are the desktop's, drawn by the desktop's code with the
//! desktop's flash guard over them ([`viz::draw`]): no setting here turns
//! the guard off or loosens its limits.

use crate::amp::analyzer::Analyzer;
use crate::amp::engine::{self, Art};
use crate::amp::meters::Meters;
use crate::amp::state::VisualOptions;
use crate::amp::viz::{self, VizState};
use crate::gesture::{self, Step, TapAction, Taps};
use crate::platform::{self, Event};
use crate::player::Player;
use crate::watch;
use eframe::egui;
use egui::{
    Align2, Color32, FontId, Id, Pos2, Rect, RichText, Sense, Stroke, Ui, UiBuilder, Vec2, pos2,
    vec2,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The least a touch target measures, in points (dp).
pub const TOUCH: f32 = 48.0;
/// The transport's buttons are larger: they are hit without looking.
const BUTTON: f32 = 64.0;
// (no target smaller than Android's 48 dp: checked when this is compiled)
const _: () = assert!(TOUCH >= 48.0 && BUTTON >= TOUCH);
/// Seconds the controls stay after the last touch while music plays.
const CONTROLS_STAY: f64 = 6.0;
/// What the buttons beside play skip, in seconds (the desktop's 15).
const BUTTON_SKIP: f64 = 15.0;
/// How often a wind or a drag of the seek bar is heard (seconds between
/// the seeks it sends).
const SCRUB_EVERY: f64 = 0.3;
/// How long a double tap's "+10 s" stays on the picture.
const HINT_STAYS: f64 = 0.7;
/// The face shown first.
const FIRST_FACE: &str = "spectrum";
/// The loudness meter's target (LUFS), the desktop's default.
const LOUDNESS_TARGET: f32 = -23.0;

const TEXT: Color32 = Color32::from_rgb(0xec, 0xf0, 0xf1);
const DIM: Color32 = Color32::from_rgb(0xa9, 0xb4, 0xbc);
const ACCENT: Color32 = Color32::from_rgb(0x2e, 0xcc, 0x71);

/// How the screen is started.
pub struct Config {
    /// The app's own folder for the documents being played (None: the host).
    pub cache: Option<PathBuf>,
    /// Tracks to play at once (the desktop preview's command line).
    pub tracks: Vec<String>,
    /// The app's files (the GL marker, see [`crate::backend::mark_gl`]).
    pub data: Option<PathBuf>,
    /// The window draws with Vulkan.
    pub vulkan: bool,
}

/// How long the GPU's measurement may take before the faces stay on the
/// CPU (it waits on the GPU: a driver that stalls must not hold up the
/// start - Android waits on this thread).
const GPU_CHECK_TIME: f64 = 8.0;

/// What the GPU's measurement found, on its thread (see [`check_gpu`]).
struct GpuCheck {
    choice: viz::gpu::Choice,
    /// The faces' shaders did not build here.
    error: Option<String>,
    took: std::time::Duration,
}

/// The graphics device was lost (the driver reset it): the faces are drawn
/// on the CPU from then on, and the next start draws with OpenGL ES.
static GPU_LOST: AtomicBool = AtomicBool::new(false);

pub struct Screen {
    player: Player,
    /// (shared with what rests while the app is hidden: see [`crate::rest`])
    meters: Arc<Mutex<Option<Meters>>>,
    rest: Arc<crate::rest::Rest>,
    /// The GPU's measurement under way: its answer, and when it began.
    gpu_check: Option<(crossbeam_channel::Receiver<GpuCheck>, f64)>,
    gpu_lost_told: bool,
    analyzer: Analyzer,
    viz_state: VizState,
    options: VisualOptions,
    tap: Vec<f32>,
    face: &'static str,
    scheme: viz::Scheme,
    cover: Option<egui::TextureHandle>,
    overview: Option<(String, Arc<Mutex<viz::Overview>>, Arc<AtomicBool>)>,
    last_frame: Option<f64>,
    /// The safe insets in pixels: left, top, right, bottom.
    insets: [i32; 4],
    controls: bool,
    faces_open: bool,
    /// When the screen was touched last (egui's clock).
    touched: f64,
    drag_from: Option<Pos2>,
    /// Where the finger was last seen during the drag.
    drag_to: Option<Pos2>,
    /// The seek bar's knob while a finger holds it (0..1), and when the
    /// sound followed it last.
    seeking: Option<f32>,
    scrubbed: f64,
    /// Taps on the picture: single (the controls) or double (a skip).
    taps: Taps,
    /// A double tap's skip, shown on its side a moment: the seconds so far,
    /// the side (true: right), until when.
    hint: Option<(f64, bool, f64)>,
    /// A long press on previous or next winding: back (-1) or on (1), from
    /// where, to where now, and when the sound followed last.
    winding: Option<Wind>,
    /// That press wound: its release is no click on the button.
    wound: Option<f64>,
    /// The pick the batches come from (a newer pick: a new list).
    pick: i32,
    picking: bool,
    /// A line of the screen's own (the picker's answer).
    note: String,
    screen_kept_on: bool,
    frames: u64,
    /// The activity was shown again: the next frame says so in the log
    /// (the window's surface was made anew and drawn into).
    shown_again: bool,
}

impl Screen {
    pub fn new(cc: &eframe::CreationContext<'_>, config: Config) -> Screen {
        let ctx = cc.egui_ctx.clone();
        style(&ctx);
        let wake = {
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        };
        platform::set_waker(wake.clone());
        let mut player = Player::start(config.cache, platform::open_fd, false, wake);
        let meters = Arc::new(Mutex::new(Some(Meters::start(player.shared().clone()))));
        // hidden for a while: the engine ends (its output stream closed),
        // the meters stop - nothing keeps a hidden app's phone awake
        let rest = {
            let transport = player.transport.clone();
            let meters = meters.clone();
            crate::rest::Rest::new(crate::rest::REST_AFTER, move || {
                transport.quit();
                meters
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
            })
        };
        {
            let transport = player.transport.clone();
            let rest = rest.clone();
            platform::set_interrupt_hook(move |what| {
                transport.interrupt(what);
                if what == platform::Interrupt::Hidden {
                    watch::set_quiet(true);
                    rest.hidden();
                }
            });
        }
        let mut viz_state = VizState::default();
        // The pixel faces' shaders run on the GPU where that is cheaper than
        // a CPU thread's drawing (the desktop's own measurement), small
        // where the GPU is slow - an emulator's software renderer. The
        // measurement waits on the GPU, so it runs on a thread of its own:
        // the faces are on the CPU until it is done.
        viz_state.set_gpu(false);
        let gpu_check = cc.wgpu_render_state.as_ref().map(|render_state| {
            guard_gpu(render_state, config.data.clone(), config.vulkan);
            let info = render_state.adapter.get_info();
            platform::log_line(&format!(
                "renderer: {:?} on {} ({:?})",
                info.backend, info.name, info.device_type
            ));
            (check_gpu(render_state.clone()), ctx.input(|i| i.time))
        });
        if gpu_check.is_none() {
            platform::log_line("renderer: no wgpu; shader faces on the CPU");
        }
        if !config.tracks.is_empty() {
            let tracks = config
                .tracks
                .iter()
                .map(|path| crate::amp::state::Track {
                    path: path.clone(),
                    ..Default::default()
                })
                .collect();
            player.set_tracks(tracks, true);
            platform::ask_audio_focus();
        }
        Screen {
            player,
            meters,
            rest,
            gpu_check,
            gpu_lost_told: false,
            analyzer: Analyzer::new(),
            viz_state,
            options: VisualOptions::default(),
            tap: Vec::new(),
            face: viz::known(FIRST_FACE),
            scheme: viz::scheme("classic", &BTreeMap::new()),
            cover: None,
            overview: None,
            last_frame: None,
            insets: [0; 4],
            controls: true,
            faces_open: false,
            touched: 0.0,
            drag_from: None,
            drag_to: None,
            seeking: None,
            scrubbed: 0.0,
            taps: Taps::default(),
            hint: None,
            winding: None,
            wound: None,
            pick: 0,
            picking: false,
            note: String::new(),
            screen_kept_on: false,
            frames: 0,
            shown_again: false,
        }
    }

    // -- what happens without a frame's picture ------------------------------------------------

    /// The system's and the engine's news. Also called while the activity
    /// is hidden (eframe's `logic`), when no picture is drawn.
    fn pump(&mut self, ctx: &egui::Context) {
        for event in platform::poll() {
            match event {
                Event::Insets(insets) => self.insets = insets,
                Event::Picked {
                    tracks,
                    pick,
                    more,
                    total,
                } => {
                    // (a batch of an older pick: of no use any more)
                    if pick < self.pick {
                        continue;
                    }
                    let first = pick != self.pick;
                    self.pick = pick;
                    self.picking = false;
                    if first && tracks.is_empty() && !more {
                        self.note = if total == 0 {
                            "Nothing to play was chosen.".into()
                        } else {
                            String::new()
                        };
                    } else if first {
                        self.note.clear();
                        // (it plays at once; the focus is asked for on the
                        // side, and if another app keeps it, it pauses)
                        self.player.set_picked(tracks, true);
                        platform::ask_audio_focus();
                        // (the controls stay a moment over the new music)
                        self.touched = ctx.input(|i| i.time);
                        self.controls = true;
                    } else if !tracks.is_empty() {
                        let play = self.player.transport.wants_playing();
                        self.player.add_picked(tracks, play);
                    }
                    if !more {
                        platform::log_line(&format!(
                            "pick {pick}: {} tracks in the list",
                            self.player.playlist.tracks.len()
                        ));
                    }
                }
                Event::NoPicker => {
                    self.picking = false;
                    self.note = "This device has no app to pick documents with.".into();
                }
                // (the music was paused by the interrupt hook already)
                Event::Shown(shown) => {
                    self.shown_again = shown && self.frames > 0;
                    if shown {
                        watch::set_quiet(false);
                        // (it rested: a new engine, the track where it was)
                        if self.rest.shown() {
                            self.player.new_engine();
                            *self.meters.lock().unwrap_or_else(|p| p.into_inner()) =
                                Some(Meters::start(self.player.shared().clone()));
                            platform::log_line("shown again: a new engine and meters");
                        }
                    }
                }
                Event::Focus(granted) => {
                    if !granted && self.player.transport.wants_playing() {
                        self.player.pause();
                        self.note = "Another app is using the sound.".into();
                    }
                }
                Event::Interrupted(_) => {}
                Event::Media(command) => self.media(command),
            }
        }
        let changed = self.player.pump();
        if changed.track {
            self.analyzer.new_track();
            if let Some(meters) = self
                .meters
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
            {
                meters.new_track();
            }
            self.show_art(ctx);
        }
        // the screen stays on while music plays under the picture
        let keep_on = self.player.playing();
        if keep_on != self.screen_kept_on {
            self.screen_kept_on = keep_on;
            platform::keep_screen_on(keep_on);
        }
    }

    /// A media command: the lock screen's, a headset's (step 2 sends them).
    fn media(&mut self, command: i32) {
        match command {
            platform::MEDIA_PLAY if !self.player.transport.wants_playing() => self.toggle(),
            platform::MEDIA_PAUSE | platform::MEDIA_STOP => self.player.pause(),
            platform::MEDIA_TOGGLE => self.toggle(),
            platform::MEDIA_NEXT => self.player.next(false),
            platform::MEDIA_PREVIOUS => self.player.previous(),
            platform::MEDIA_FORWARD => self.player.skip(BUTTON_SKIP),
            platform::MEDIA_REWIND => self.player.skip(-BUTTON_SKIP),
            _ => {}
        }
    }

    fn toggle(&mut self) {
        if self.player.transport.wants_playing() {
            self.player.pause();
            platform::abandon_audio_focus();
        } else {
            self.note.clear();
            self.player.toggle();
            platform::ask_audio_focus();
        }
    }

    /// The GPU's measurement, once it is done (or given up on), and a lost
    /// device.
    fn follow_gpu(&mut self, now: f64) {
        if GPU_LOST.load(Ordering::Relaxed) && !self.gpu_lost_told {
            self.gpu_lost_told = true;
            self.gpu_check = None;
            self.viz_state.set_gpu(false);
            self.note = "The graphics driver was reset: please start the app again.".into();
        }
        let Some((answer, began)) = &self.gpu_check else {
            return;
        };
        match answer.try_recv() {
            Ok(check) => {
                self.gpu_check = None;
                if let Some(error) = check.error {
                    log::error!(target: "axrossamp", "the faces' shaders do not build here: {error}; the faces stay on the CPU");
                    return;
                }
                let choice = check.choice;
                self.viz_state.set_gpu(choice.gpu);
                if choice.gpu {
                    self.viz_state.gpu.ms_per_pixel = choice.gpu_ms_per_pixel;
                }
                let slow = choice.slow_for(choice.gpu);
                if slow {
                    self.viz_state.gpu.max_pixels = 90_000.0;
                }
                platform::log_line(&format!(
                    "shader faces on the {}{} (measured in {} ms)",
                    if choice.gpu { "GPU" } else { "CPU" },
                    if slow { ", small (slow here)" } else { "" },
                    check.took.as_millis()
                ));
            }
            Err(crossbeam_channel::TryRecvError::Empty) if now - *began > GPU_CHECK_TIME => {
                self.gpu_check = None;
                log::warn!(target: "axrossamp", "the GPU's measurement takes longer than {GPU_CHECK_TIME} s: the faces stay on the CPU");
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
            Err(crossbeam_channel::TryRecvError::Disconnected) => self.gpu_check = None,
        }
    }

    /// The track's cover: its texture (the cover face), its small picture
    /// (the flash guard's estimate of it).
    fn show_art(&mut self, ctx: &egui::Context) {
        let art: Option<Arc<Art>> = self.player.info.art.clone();
        self.cover = art.as_deref().map(|art| {
            let size = [art.width as usize, art.height as usize];
            let image = egui::ColorImage::from_rgba_unmultiplied(size, &art.rgba);
            ctx.load_texture("cover", image, egui::TextureOptions::LINEAR)
        });
        self.viz_state.cover_thumb = art.as_deref().map(|art| {
            Arc::new(viz::Thumb::from_rgba(
                art.width as usize,
                art.height as usize,
                &art.rgba,
            ))
        });
    }

    /// The track's waveform for the overview face, read on a thread of its
    /// own; another track's reading stops.
    fn read_overview(&mut self) {
        let path = self
            .player
            .current_path()
            .filter(|_| self.player.ready() && self.player.duration.is_some())
            .map(str::to_owned);
        let Some(path) = path else {
            if let Some((_, _, stop)) = self.overview.take() {
                stop.store(true, Ordering::Relaxed);
            }
            return;
        };
        if self.overview.as_ref().is_some_and(|(p, _, _)| *p == path) {
            return;
        }
        if let Some((_, _, stop)) = self.overview.take() {
            stop.store(true, Ordering::Relaxed);
        }
        let shared = Arc::new(Mutex::new(viz::Overview::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (into, flag, file) = (shared.clone(), stop.clone(), path.clone());
        let _ = std::thread::Builder::new()
            .name("overview".into())
            .spawn(move || {
                let _ = engine::waveform(&file, 1600, &flag, |columns, done| {
                    if let Ok(mut o) = into.lock() {
                        o.columns.clear();
                        o.columns.extend_from_slice(columns);
                        o.done = done;
                    }
                });
            });
        self.overview = Some((path, shared, stop));
    }

    // -- the picture ---------------------------------------------------------------------------

    /// The analysis of this frame's sound, then the face into `rect`: as the
    /// desktop's window does it.
    fn draw_face(&mut self, ui: &Ui, rect: Rect) {
        let (now, stable) = ui.input(|i| (i.time, i.stable_dt));
        let since = match self.last_frame {
            Some(last) => (now - last) as f32,
            None => stable,
        };
        let dt = since.clamp(0.0, 0.2);
        // (the music stopped: the picture falls to rest in the clock's seconds)
        let fall_dt = since.clamp(0.0, 2.0);
        self.last_frame = Some(now);
        let shared = self.player.shared().clone();
        let written = shared.tap_into(&mut self.tap);
        let rate = shared.device_rate.load(Ordering::Relaxed);
        let playing = self.player.playing();
        // (paused: the picture stands as it was, not emptied)
        let frozen = !playing && self.player.ready() && self.player.position() > 0.05;
        self.analyzer.motion = crate::amp::analyzer::guard::Motion::Calm;
        self.analyzer.calm = viz::scheme_calm(&self.scheme);
        if !frozen {
            let readings = self
                .meters
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
                .map(|m| m.read());
            if let Some(readings) = readings {
                self.analyzer.set_readings(readings);
            }
            self.analyzer.update(
                &self.tap,
                written,
                rate,
                &self.options,
                if playing { dt } else { fall_dt },
                playing,
                viz::needs_now(self.face, &self.viz_state),
            );
        }
        if self.face == "overview" {
            self.read_overview();
        }
        let overview = self
            .overview
            .as_ref()
            .filter(|_| self.face == "overview")
            .map(|(_, o, _)| o.clone());
        let overview = overview.as_ref().and_then(|o| o.lock().ok());
        let title = self.player.title();
        let frame = viz::Frame {
            painter: ui.painter(),
            rect,
            a: &self.analyzer,
            s: &self.scheme,
            dt: if frozen { 0.0 } else { dt },
            title: &title,
            overview: overview.as_deref(),
            position: self.player.position() as f32,
            duration: self.player.duration.unwrap_or(0.0) as f32,
            // (no pointer rests over a touch screen)
            hover: None,
            opacity: 1.0,
            second: false,
            loudness_target: LOUDNESS_TARGET,
            cover: self
                .cover
                .as_ref()
                .filter(|_| self.face == "cover")
                .map(|t| (t.id(), t.size_vec2())),
        };
        self.viz_state.gpu.interval = 1.0 / self.options.fps.max(1) as f32;
        viz::draw(&frame, self.face, &mut self.viz_state);
    }

    /// The display without what the system covers, in points.
    fn safe(&self, screen: Rect, pixels_per_point: f32) -> Rect {
        let [left, top, right, bottom] = self.insets.map(|px| px as f32 / pixels_per_point);
        safe_rect(screen, [left, top, right, bottom])
    }

    fn set_face(&mut self, key: &'static str) {
        self.face = key;
    }

    // -- the controls --------------------------------------------------------------------------

    fn top_bar(&mut self, ui: &mut Ui, screen: Rect, safe: Rect) {
        let bar = Rect::from_min_max(safe.min, pos2(safe.max.x, safe.min.y + TOUCH + 16.0));
        // (the shade reaches up under the status bar)
        shade(
            ui,
            Rect::from_min_max(screen.min, pos2(screen.max.x, bar.max.y)),
            true,
        );
        let row = bar.shrink2(vec2(12.0, 8.0));
        let width = ((row.width() - 16.0) / 3.0).clamp(72.0, 168.0);
        let faces = Rect::from_min_size(row.min, vec2(width, TOUCH));
        let files = Rect::from_min_size(pos2(row.max.x - width, row.min.y), vec2(width, TOUCH));
        let folder = files.translate(vec2(-(width + 8.0), 0.0));
        if text_button(ui, faces, "faces", "Faces", self.faces_open).clicked() {
            self.faces_open = !self.faces_open;
        }
        if text_button(ui, folder, "folder", "Folder", false).clicked() && !self.picking {
            self.picking = true;
            self.note.clear();
            platform::pick_folder();
        }
        if text_button(ui, files, "files", "Files", false).clicked() && !self.picking {
            self.picking = true;
            self.note.clear();
            platform::pick_files();
        }
    }

    fn bottom_panel(&mut self, ui: &mut Ui, screen: Rect, safe: Rect) {
        let height = 28.0 + 22.0 + TOUCH + BUTTON + 28.0;
        let panel = Rect::from_min_max(pos2(safe.min.x, safe.max.y - height), safe.max);
        shade(
            ui,
            Rect::from_min_max(pos2(screen.min.x, panel.min.y - 24.0), screen.max),
            false,
        );
        let inner = panel.shrink2(vec2(16.0, 8.0));
        // the title, and under it the artist and album - or what to say
        let title = self.player.title();
        let (first, second) = if !self.player.has_tracks() {
            (
                "Pick a folder or files to play".to_owned(),
                "Swipe for the next picture, tap to hide this.".to_owned(),
            )
        } else {
            (title, self.player.byline())
        };
        let second = [&self.note, &self.player.status, &second]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_default();
        let line = Rect::from_min_size(inner.min, vec2(inner.width(), 28.0));
        one_line(ui, line, &first, 19.0, TEXT);
        let line = line.translate(vec2(0.0, 28.0));
        let line = Rect::from_min_size(line.min, vec2(inner.width(), 22.0));
        one_line(ui, line, &second, 14.0, DIM);

        // the seek bar between the time gone and the track's length
        let row = Rect::from_min_size(pos2(inner.min.x, line.max.y), vec2(inner.width(), TOUCH));
        let duration = self.player.duration.unwrap_or(0.0).max(0.0);
        let position = self.player.shown_position().clamp(0.0, duration.max(0.0));
        let can_seek = self.player.ready() && self.player.info.seekable && duration > 0.0;
        let shown = match (self.seeking, &self.winding) {
            (Some(at), _) => at as f64 * duration,
            (None, Some(wind)) => wind.to.clamp(0.0, duration.max(0.0)),
            (None, None) => position,
        };
        let clock = 52.0;
        let font = FontId::monospace(13.0);
        ui.painter().text(
            pos2(row.min.x, row.center().y),
            Align2::LEFT_CENTER,
            time_text(shown),
            font.clone(),
            DIM,
        );
        ui.painter().text(
            pos2(row.max.x, row.center().y),
            Align2::RIGHT_CENTER,
            if duration > 0.0 {
                time_text(duration)
            } else {
                "–:––".to_owned()
            },
            font,
            DIM,
        );
        let bar = Rect::from_min_max(
            pos2(row.min.x + clock, row.min.y),
            pos2(row.max.x - clock, row.max.y),
        );
        let fraction = if duration > 0.0 {
            (shown / duration) as f32
        } else {
            0.0
        };
        let (now_at, released) = seek_bar(ui, bar, self.seeking.unwrap_or(fraction), can_seek);
        let now = ui.input(|i| i.time);
        if can_seek {
            if let Some(at) = now_at {
                // (dragged: the sound follows the knob now and then - a
                // scrub - and the clock shows where it is)
                if self.seeking.is_none() {
                    self.scrubbed = now;
                }
                self.seeking = Some(at);
                if now - self.scrubbed >= SCRUB_EVERY {
                    self.scrubbed = now;
                    self.player.wind_to(at as f64 * duration);
                }
            }
            // (let go: as the bar says, or - a long press that was neither a
            // tap nor a drag - simply no finger down any more)
            let let_go = released || !ui.input(|i| i.pointer.any_down());
            if let_go && let Some(at) = self.seeking.take() {
                self.player.wind_to(at as f64 * duration);
            }
        } else {
            self.seeking = None;
        }

        // previous, 15 s back, play or pause, 15 s on, next: five round
        // buttons, as large as the width lets them be (never under 48)
        let centre = pos2(inner.center().x, row.max.y + 4.0 + BUTTON / 2.0);
        let size = ((inner.width() - 4.0 * 8.0) / 5.0).clamp(TOUCH, BUTTON);
        let step = size + ((inner.width() - 5.0 * size) / 4.0).clamp(8.0, 24.0);
        let enabled = self.player.has_tracks();
        let at =
            |k: f32| Rect::from_center_size(pos2(centre.x + k * step, centre.y), Vec2::splat(size));
        let previous = icon_button(ui, at(-2.0), "previous", Icon::Previous, enabled);
        if self.hold(ui, &previous, -1.0, now) {
            self.player.previous();
        }
        if icon_button(ui, at(-1.0), "back", Icon::Back, enabled).clicked() {
            self.player.skip(-BUTTON_SKIP);
        }
        let icon = if self.player.transport.wants_playing() {
            Icon::Pause
        } else {
            Icon::Play
        };
        if icon_button(ui, at(0.0), "play", icon, enabled).clicked() {
            self.toggle();
        }
        if icon_button(ui, at(1.0), "on", Icon::On, enabled).clicked() {
            self.player.skip(BUTTON_SKIP);
        }
        let next = icon_button(ui, at(2.0), "next", Icon::Next, enabled);
        if self.hold(ui, &next, 1.0, now) {
            self.player.next(false);
        }
    }

    /// Previous or next (`direction` -1 or 1) under a finger: a long press
    /// winds back or on while it is held, faster the longer
    /// ([`gesture::wind_speed`]); true: it was a tap (the button's own
    /// step).
    fn hold(&mut self, ui: &Ui, button: &egui::Response, direction: f64, now: f64) -> bool {
        let down = button.is_pointer_button_down_on();
        if down {
            let pressed = ui.input(|i| i.pointer.press_start_time()).unwrap_or(now);
            let held = now - pressed;
            if gesture::wind_speed(held) > 0.0 && self.player.has_tracks() {
                if self
                    .winding
                    .as_ref()
                    .is_none_or(|w| w.direction != direction)
                {
                    self.winding = Some(Wind {
                        direction,
                        from: self.player.shown_position(),
                        to: self.player.shown_position(),
                        heard: now,
                    });
                    self.wound = Some(direction);
                }
                if let Some(wind) = self.winding.as_mut() {
                    wind.to = wind.from + direction * gesture::wound(held);
                    if now - wind.heard >= SCRUB_EVERY {
                        wind.heard = now;
                        let to = wind.to;
                        self.player.wind_to(to);
                    }
                }
                ui.ctx().request_repaint();
            }
        } else if let Some(wind) = self.winding.take_if(|w| w.direction == direction) {
            // (let go: where it was wound to)
            self.player.wind_to(wind.to);
        }
        if button.clicked() {
            // (the release of a press that wound is no step)
            return self.wound.take() != Some(direction);
        }
        if !down
            && !button.is_pointer_button_down_on()
            && self.winding.is_none()
            && !ui.input(|i| i.pointer.any_down())
        {
            self.wound = None;
        }
        false
    }

    /// A double tap's skip: done, and shown on its side a moment (a chain
    /// of them adds up).
    fn skip_by_tap(&mut self, secs: f64, now: f64) {
        if !self.player.has_tracks() {
            return;
        }
        self.player.skip(secs);
        let right = secs > 0.0;
        let total = match self.hint {
            Some((so_far, side, until)) if side == right && now <= until => so_far + secs,
            _ => secs,
        };
        self.hint = Some((total, right, now + HINT_STAYS));
    }

    /// The double tap's "+10 s" over its side of the picture.
    fn draw_hint(&mut self, ui: &Ui, screen: Rect, now: f64) {
        let Some((secs, right, until)) = self.hint else {
            return;
        };
        if now > until {
            self.hint = None;
            return;
        }
        // (fading in its last third; never a flash: soft, small, grey on
        // the dark picture, and the flash guard is over the faces anyway)
        let left = ((until - now) / (HINT_STAYS / 3.0)).min(1.0) as f32;
        let x = if right {
            screen.max.x - screen.width() / 6.0
        } else {
            screen.min.x + screen.width() / 6.0
        };
        let centre = pos2(x, screen.center().y);
        let painter = ui.painter();
        painter.circle_filled(
            centre,
            44.0,
            Color32::from_rgba_unmultiplied(12, 16, 20, (150.0 * left) as u8),
        );
        painter.text(
            centre,
            Align2::CENTER_CENTER,
            format!("{}{:.0} s", if secs > 0.0 { "+" } else { "−" }, secs.abs()),
            FontId::proportional(18.0),
            Color32::from_rgba_unmultiplied(0xec, 0xf0, 0xf1, (230.0 * left) as u8),
        );
        ui.ctx().request_repaint();
    }

    /// The list of faces, group by group; a tap chooses one and closes it.
    fn face_list(&mut self, ui: &mut Ui, safe: Rect) {
        let top = safe.min.y + TOUCH + 16.0;
        let area = Rect::from_min_max(pos2(safe.min.x, top), safe.max).shrink2(vec2(12.0, 8.0));
        ui.painter()
            .rect_filled(area, 12.0, Color32::from_rgba_unmultiplied(12, 16, 20, 242));
        // (a touch on the list is not a touch on the picture under it)
        ui.interact(
            area,
            Id::unique("face-list-ground"),
            Sense::click_and_drag(),
        );
        let mut chosen = None;
        ui.scope_builder(UiBuilder::new().max_rect(area.shrink(8.0)), |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let width = ui.available_width();
                    let mut group = None;
                    for face in viz::FACES.iter().filter(|f| f.group != viz::Group::Off) {
                        if group != Some(face.group) {
                            group = Some(face.group);
                            ui.add_space(8.0);
                            ui.label(RichText::new(face.group.name()).size(14.0).color(ACCENT));
                            ui.add_space(2.0);
                        }
                        let text = RichText::new(face.name).size(17.0).color(TEXT);
                        let button = egui::Button::new(text)
                            .selected(face.key == self.face)
                            .min_size(vec2(width, TOUCH + 4.0));
                        if ui.add(button).clicked() {
                            chosen = Some(face.key);
                        }
                    }
                    ui.add_space(8.0);
                });
        });
        if let Some(key) = chosen {
            self.set_face(key);
            self.faces_open = false;
        }
    }
}

impl eframe::App for Screen {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        watch::doing("amp-window", "reading the news");
        self.pump(ctx);
        watch::done("amp-window");
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        watch::doing("amp-window", "drawing a frame");
        self.frame(ui);
        watch::done("amp-window");
    }
}

impl Screen {
    fn frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.follow_gpu(ui.input(|i| i.time));
        let (screen, now, touching) = ui.input(|i| {
            (
                i.viewport_rect(),
                i.time,
                i.pointer.any_down() || i.pointer.any_released(),
            )
        });
        if touching {
            self.touched = now;
        }
        let safe = self.safe(screen, ctx.pixels_per_point());
        // Back: out of the list of faces, else out of the app (to the home
        // screen; the activity stays as it is)
        if ui.input(|i| i.key_pressed(egui::Key::BrowserBack) || i.key_pressed(egui::Key::Escape)) {
            if self.faces_open {
                self.faces_open = false;
            } else {
                platform::leave();
            }
        }
        ui.painter().rect_filled(screen, 0.0, Color32::BLACK);

        // the picture, and a finger on it: a tap shows or hides the
        // controls, a swipe to the side steps through the faces
        self.draw_face(ui, screen);
        let picture = ui.interact(screen, Id::unique("picture"), Sense::click_and_drag());
        if self.faces_open {
            if picture.clicked() {
                self.faces_open = false;
            }
        } else {
            if picture.drag_started() {
                // (from where the finger went down, not from where egui
                // first called it a drag)
                self.drag_from = ui
                    .input(|i| i.pointer.press_origin())
                    .or(picture.interact_pointer_pos());
                self.drag_to = None;
            }
            if picture.dragged() {
                self.drag_to = picture.interact_pointer_pos().or(self.drag_to);
            }
            if picture.drag_stopped()
                && let (Some(from), Some(to)) = (
                    self.drag_from.take(),
                    ui.input(|i| i.pointer.interact_pos())
                        .or(self.drag_to.take()),
                )
            {
                match gesture::swipe(to.x - from.x, to.y - from.y, screen.width()) {
                    Some(Step::Next) => self.set_face(viz::next(self.face)),
                    Some(Step::Previous) => self.set_face(viz::previous(self.face)),
                    None => {}
                }
            }
            // a tap: the controls (at once in the middle third, a moment
            // later on the sides), a double tap on a side: a skip
            let mut actions = Vec::new();
            if picture.clicked()
                && let Some(at) = picture.interact_pointer_pos()
            {
                actions.extend(
                    self.taps
                        .tap(now, at.x - screen.min.x, at.y, screen.width()),
                );
            }
            actions.extend(self.taps.tick(now));
            for action in actions {
                match action {
                    TapAction::Controls => self.controls = !self.controls,
                    TapAction::Skip(secs) => self.skip_by_tap(secs, now),
                }
            }
            if self.taps.waiting() {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        self.draw_hint(ui, screen, now);

        // the controls leave by themselves while music plays - never while
        // there is nothing to play yet, a finger is on them or the list is open
        if self.controls
            && self.player.playing()
            && !self.faces_open
            && self.seeking.is_none()
            && self.winding.is_none()
            && now - self.touched > CONTROLS_STAY
        {
            self.controls = false;
        }
        if self.controls || self.faces_open {
            self.top_bar(ui, screen, safe);
        }
        if self.faces_open {
            self.face_list(ui, safe);
        } else if self.controls {
            self.bottom_panel(ui, screen, safe);
        }

        self.frames += 1;
        if self.frames == 1 {
            // (the line the emulator's smoke test waits for)
            platform::log_line(&format!(
                "first frame: {:.0} x {:.0} points at {:.2} pixels a point, face {}",
                screen.width(),
                screen.height(),
                ctx.pixels_per_point(),
                self.face,
            ));
        }
        if std::mem::take(&mut self.shown_again) {
            platform::log_line(&format!(
                "frame after resume: {:.0} x {:.0} points",
                screen.width(),
                screen.height(),
            ));
        }
        // the picture moves with the music: frame after frame while it
        // plays (and while it falls to rest after), else only when touched
        if self.player.playing() || self.analyzer.bars.iter().any(|b| *b > 0.001) {
            let fps = self.options.fps.max(1) as f32;
            ctx.request_repaint_after(std::time::Duration::from_secs_f32(1.0 / fps));
        }
    }
}

/// wgpu's errors are logged instead of ending the app (wgpu's own handler
/// panics at the first one), and a lost device is told: the faces go to
/// the CPU, and where it was Vulkan the next start draws with OpenGL ES.
fn guard_gpu(render_state: &eframe::egui_wgpu::RenderState, data: Option<PathBuf>, vulkan: bool) {
    use std::sync::atomic::AtomicUsize;
    let told = Arc::new(AtomicUsize::new(0));
    render_state
        .device
        .on_uncaptured_error(Arc::new(move |error: eframe::wgpu::Error| {
            // (the first few in full; then one now and then)
            let n = told.fetch_add(1, Ordering::Relaxed);
            if n < 10 || n.is_power_of_two() {
                log::error!(target: "axrossamp", "wgpu error ({}): {error}", n + 1);
            }
        }));
    render_state
        .device
        .set_device_lost_callback(move |reason, message| {
            log::error!(target: "axrossamp", "the graphics device was lost ({reason:?}): {message}");
            GPU_LOST.store(true, Ordering::Relaxed);
            if vulkan && let Some(dir) = data.as_deref() {
                crate::backend::mark_gl(dir, &format!("the device was lost: {message}"));
            }
        });
}

/// The faces' GPU resources and the GPU's measurement, on a thread of
/// their own (both wait on the GPU and its driver), inside an error scope:
/// shaders this GPU's driver does not take are an answer, not a crash.
fn check_gpu(
    render_state: eframe::egui_wgpu::RenderState,
) -> crossbeam_channel::Receiver<GpuCheck> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let _ = std::thread::Builder::new()
        .name("amp-gpu".into())
        .spawn(move || {
            let began = std::time::Instant::now();
            let scope = render_state
                .device
                .push_error_scope(eframe::wgpu::ErrorFilter::Validation);
            viz::gpu::install(&render_state);
            let choice = viz::gpu::choose(&render_state);
            let error = crate::backend::block_on(scope.pop()).map(|e| e.to_string());
            let _ = tx.send(GpuCheck {
                choice,
                error,
                took: began.elapsed(),
            });
        });
    rx
}

/// `screen` without the insets (left, top, right, bottom, in points); never
/// smaller than nothing, whatever the insets say.
pub fn safe_rect(screen: Rect, insets: [f32; 4]) -> Rect {
    let [left, top, right, bottom] = insets.map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
    let left = left.min(screen.width() / 2.0);
    let right = right.min(screen.width() / 2.0);
    let top = top.min(screen.height() / 2.0);
    let bottom = bottom.min(screen.height() / 2.0);
    Rect::from_min_max(
        pos2(screen.min.x + left, screen.min.y + top),
        pos2(screen.max.x - right, screen.max.y - bottom),
    )
}

/// m:ss, or h:mm:ss from an hour on.
pub fn time_text(secs: f64) -> String {
    let secs = if secs.is_finite() {
        secs.max(0.0) as u64
    } else {
        0
    };
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Text and touch sizes for a phone; dark, as the picture's ground is.
fn style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.spacing.interact_size = vec2(TOUCH, TOUCH);
        style.spacing.button_padding = vec2(16.0, 12.0);
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.scroll.bar_width = 6.0;
        style.interaction.selectable_labels = false;
    });
}

/// A dark shade behind the controls: the picture stays visible, the text
/// readable on any face. `down`: darkest at the top (the top bar's).
fn shade(ui: &Ui, rect: Rect, down: bool) {
    let dark = Color32::from_rgba_unmultiplied(0, 0, 0, 200);
    let light = Color32::from_rgba_unmultiplied(0, 0, 0, 110);
    let (top, bottom) = if down { (dark, light) } else { (light, dark) };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
    // (a touch on the controls' ground is not a tap on the picture)
    ui.interact(rect, Id::unique(("shade", down)), Sense::click());
}

/// One line of text in `rect`, cut short with an ellipsis where it is longer.
fn one_line(ui: &mut Ui, rect: Rect, text: &str, size: f32, colour: Color32) {
    if text.is_empty() {
        return;
    }
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        ui.add(egui::Label::new(RichText::new(text).size(size).color(colour)).truncate());
    });
}

/// A button with a word on it, at least a touch target high.
fn text_button(ui: &mut Ui, rect: Rect, id: &str, label: &str, on: bool) -> egui::Response {
    let response = ui.interact(rect, Id::unique(("button", id)), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let fill = if response.is_pointer_button_down_on() || on {
        Color32::from_rgba_unmultiplied(0x2e, 0xcc, 0x71, 70)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 28)
    };
    // (drawn a little smaller than it can be hit)
    ui.painter()
        .rect_filled(rect.shrink2(vec2(0.0, 4.0)), 10.0, fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(16.0),
        TEXT,
    );
    response
}

/// A long press winding (see [`Screen::hold`]).
struct Wind {
    direction: f64,
    from: f64,
    to: f64,
    heard: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Icon {
    Play,
    Pause,
    Previous,
    Next,
    /// 15 s back, 15 s on.
    Back,
    On,
}

impl Icon {
    fn label(self) -> &'static str {
        match self {
            Icon::Play => "Play",
            Icon::Pause => "Pause",
            Icon::Previous => "Previous track (hold: wind back)",
            Icon::Next => "Next track (hold: wind on)",
            Icon::Back => "15 seconds back",
            Icon::On => "15 seconds on",
        }
    }
}

/// A round button with a drawn sign (no font has to carry it).
fn icon_button(ui: &mut Ui, rect: Rect, id: &str, icon: Icon, enabled: bool) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, Id::unique(("button", id)), sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, icon.label()));
    let painter = ui.painter();
    let centre = rect.center();
    let main = matches!(icon, Icon::Play | Icon::Pause);
    let down = response.is_pointer_button_down_on();
    let fill = match (main, down) {
        (true, false) => Color32::from_rgba_unmultiplied(0x2e, 0xcc, 0x71, 210),
        (true, true) => ACCENT,
        (false, true) => Color32::from_rgba_unmultiplied(255, 255, 255, 60),
        (false, false) => Color32::from_rgba_unmultiplied(255, 255, 255, 24),
    };
    let radius = rect.width().min(rect.height()) / 2.0 - if main { 0.0 } else { 6.0 };
    painter.circle_filled(centre, radius, fill);
    let ink = match (enabled, main) {
        (false, _) => Color32::from_rgba_unmultiplied(255, 255, 255, 70),
        (true, true) => Color32::from_rgb(0x10, 0x14, 0x18),
        (true, false) => TEXT,
    };
    let triangle = |tip_x: f32, back_x: f32, half: f32| {
        egui::Shape::convex_polygon(
            vec![
                pos2(back_x, centre.y - half),
                pos2(tip_x, centre.y),
                pos2(back_x, centre.y + half),
            ],
            ink,
            Stroke::NONE,
        )
    };
    let bar = |x: f32, width: f32, half: f32| {
        Rect::from_min_max(pos2(x, centre.y - half), pos2(x + width, centre.y + half))
    };
    match icon {
        Icon::Play => {
            painter.add(triangle(centre.x + 13.0, centre.x - 8.0, 13.0));
        }
        Icon::Pause => {
            painter.rect_filled(bar(centre.x - 10.0, 7.0, 12.0), 1.5, ink);
            painter.rect_filled(bar(centre.x + 3.0, 7.0, 12.0), 1.5, ink);
        }
        Icon::Next => {
            painter.add(triangle(centre.x + 6.0, centre.x - 9.0, 10.0));
            painter.rect_filled(bar(centre.x + 6.0, 4.0, 10.0), 1.0, ink);
        }
        Icon::Previous => {
            painter.add(triangle(centre.x - 6.0, centre.x + 9.0, 10.0));
            painter.rect_filled(bar(centre.x - 10.0, 4.0, 10.0), 1.0, ink);
        }
        Icon::Back | Icon::On => {
            // (a ring with an arrowhead, and the seconds in it)
            let back = icon == Icon::Back;
            let r = (radius - 9.0).max(10.0);
            let (from, to) = if back {
                (-0.6f32, 4.1f32)
            } else {
                (-2.54f32, 2.17f32)
            };
            let points: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = from + (to - from) * i as f32 / 20.0;
                    pos2(centre.x + r * a.cos(), centre.y + r * a.sin())
                })
                .collect();
            let head = points[0];
            painter.add(egui::Shape::line(points, Stroke::new(2.0, ink)));
            let dx = if back { 5.0 } else { -5.0 };
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(head.x + dx, head.y - 4.0),
                    pos2(head.x - dx * 0.2, head.y - 7.0),
                    pos2(head.x - dx * 0.2, head.y + 1.0),
                ],
                ink,
                Stroke::NONE,
            ));
            painter.text(
                centre,
                Align2::CENTER_CENTER,
                "15",
                FontId::proportional(13.0),
                ink,
            );
        }
    }
    response
}

/// The seek bar: a touch target's height, a thin line with a knob. Gives
/// where a finger holds the knob now (0..1), and whether it just let go.
fn seek_bar(ui: &mut Ui, rect: Rect, fraction: f32, enabled: bool) -> (Option<f32>, bool) {
    let sense = if enabled {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, Id::unique("seek"), sense);
    response.widget_info(|| {
        egui::WidgetInfo::slider(enabled, fraction as f64, "Position in the track")
    });
    let line = Rect::from_min_max(
        pos2(rect.min.x + 12.0, rect.center().y - 2.0),
        pos2(rect.max.x - 12.0, rect.center().y + 2.0),
    );
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let knob = pos2(line.min.x + line.width() * fraction, line.center().y);
    let painter = ui.painter();
    painter.rect_filled(
        line,
        2.0,
        Color32::from_rgba_unmultiplied(255, 255, 255, 60),
    );
    if enabled {
        painter.rect_filled(
            Rect::from_min_max(line.min, pos2(knob.x, line.max.y)),
            2.0,
            ACCENT,
        );
        let radius = if response.is_pointer_button_down_on() {
            11.0
        } else {
            8.0
        };
        painter.circle_filled(knob, radius, ACCENT);
    }
    let held = (enabled && (response.dragged() || response.is_pointer_button_down_on()))
        .then(|| response.interact_pointer_pos())
        .flatten()
        .map(|p| ((p.x - line.min.x) / line.width().max(1.0)).clamp(0.0, 1.0));
    // (a tap on the bar seeks there too)
    let tapped = (enabled && response.clicked())
        .then(|| response.interact_pointer_pos())
        .flatten()
        .map(|p| ((p.x - line.min.x) / line.width().max(1.0)).clamp(0.0, 1.0));
    let released = enabled && (response.drag_stopped() || response.clicked());
    (held.or(tapped), released)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_safe_area_is_the_screen_without_the_insets() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 800.0));
        let safe = safe_rect(screen, [0.0, 32.0, 0.0, 24.0]);
        assert_eq!(
            safe,
            Rect::from_min_max(pos2(0.0, 32.0), pos2(400.0, 776.0))
        );
        // (landscape: a cutout on the left, the navigation bar on the right)
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 400.0));
        let safe = safe_rect(screen, [40.0, 0.0, 48.0, 0.0]);
        assert_eq!(
            safe,
            Rect::from_min_max(pos2(40.0, 0.0), pos2(752.0, 400.0))
        );
    }

    #[test]
    fn insets_that_make_no_sense_leave_a_rectangle() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 800.0));
        let safe = safe_rect(screen, [-10.0, f32::NAN, 9000.0, f32::INFINITY]);
        assert!(safe.width() >= 0.0 && safe.height() >= 0.0);
        assert_eq!(safe.min, pos2(0.0, 0.0));
        assert_eq!(safe.max.x, 200.0);
    }

    #[test]
    fn times_read_as_a_clock() {
        assert_eq!(time_text(0.0), "0:00");
        assert_eq!(time_text(59.9), "0:59");
        assert_eq!(time_text(61.0), "1:01");
        assert_eq!(time_text(3600.0 + 62.0), "1:01:02");
        assert_eq!(time_text(f64::NAN), "0:00");
        assert_eq!(time_text(-5.0), "0:00");
    }

    #[test]
    fn every_face_of_the_desktop_can_be_chosen() {
        // (the list shows them all but "Off"; the first face is one of them)
        let listed = viz::FACES
            .iter()
            .filter(|f| f.group != viz::Group::Off)
            .count();
        assert!(listed >= 69, "{listed}");
        assert_eq!(viz::known(FIRST_FACE), FIRST_FACE);
    }
}
