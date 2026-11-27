//! The player on a thread of its own, for as long as the process lives:
//! the music plays on while no window is there (the app hidden, the
//! screen off, the lock screen), driven by the system's media commands -
//! the notification, the lock screen, a headset, Bluetooth - as by the
//! screen's buttons.
//!
//! The window cannot drive it while hidden: on Android eframe drops the
//! window when the activity stops and calls the app no more until it is
//! shown again. So the [`Player`] lives on the `amp-player` thread; the
//! screen holds a [`Handle`] - its commands go to that thread, what it
//! shows comes from a [`View`] the thread keeps, the position and the
//! sound from the engine's own shared state.
//!
//! The thread waits for work - a command, the engine's or the opening
//! thread's news, a retry's time - and does not wake otherwise; while
//! music plays it looks every [`SAVE_EVERY`] to keep the position saved.
//! It tells the system what plays ([`platform::playback`]: the media
//! session, its notification, the playback service), keeps the state in
//! the app's files ([`crate::saved`]), and rests the engine when the app
//! has been hidden and paused for [`ENGINE_REST`].

use crate::amp::engine::{Art, Shared};
use crate::amp::state::Track;
use crate::picked::Picked;
use crate::platform::{self, ToPlayer};
use crate::player::{Changed, Player};
use crate::saved::Saved;
use crate::transport::Transport;
use crossbeam_channel::{Receiver, Sender};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// While music plays, the state is saved this often (the position).
pub const SAVE_EVERY: Duration = Duration::from_secs(15);

/// Hidden and paused this long: the engine ends, its output stream closed.
pub const ENGINE_REST: Duration = Duration::from_secs(60);

/// What the screen sends.
#[derive(Debug)]
pub enum Cmd {
    SetPicked(Vec<Picked>, bool),
    AddPicked(Vec<Picked>),
    SetTracks(Vec<Track>, bool),
    Toggle,
    Play,
    Pause,
    Next,
    Previous,
    Skip(f64),
    WindTo(f64),
    /// The window shown (true) or hidden.
    Shown(bool),
    /// From the system: see [`ToPlayer`].
    System(ToPlayer),
}

/// What the screen shows, as the player's thread last saw it.
#[derive(Clone, Default)]
pub struct View {
    pub title: String,
    pub byline: String,
    pub status: String,
    pub duration: Option<f64>,
    pub ready: bool,
    pub seekable: bool,
    pub tracks: usize,
    pub art: Option<Arc<Art>>,
    pub current_path: Option<String>,
    /// Counts the tracks heard (a new one: its picture, its cover).
    pub track: u64,
    pub seek_pending: Option<f64>,
    pub seek_asked: Option<(f64, Instant)>,
    /// The engine's shared state (another after a rest).
    pub shared: Option<Arc<Shared>>,
}

/// The screen's end: the same few methods the player has.
pub struct Handle {
    tx: Sender<Cmd>,
    view: Arc<Mutex<View>>,
    pub transport: Arc<Transport>,
    /// The engine's shared state when the handle was made (the view's
    /// after a rest).
    shared: Arc<Shared>,
    seen_track: u64,
}

/// Where the player keeps what it keeps.
pub struct Places {
    /// For the documents being played (None: plain paths only).
    pub cache: Option<PathBuf>,
    /// The app's files: the saved state (None: nothing kept).
    pub data: Option<PathBuf>,
}

impl Handle {
    /// The player's thread started (`headless`: no sound device, the
    /// tests). `wake` asks the window for a frame when the view changed.
    pub fn start(
        places: Places,
        open_fd: fn(&str) -> Option<i32>,
        headless: bool,
        wake: impl Fn() + Send + Sync + Clone + 'static,
    ) -> Handle {
        let (tx, rx) = crossbeam_channel::unbounded::<Cmd>();
        let (woke, wakes) = crossbeam_channel::unbounded::<()>();
        let player = Player::start(places.cache, open_fd, headless, move || {
            let _ = woke.try_send(());
        });
        let transport = player.transport.clone();
        let shared = player.shared().clone();
        let view = Arc::new(Mutex::new(View {
            shared: Some(shared.clone()),
            ..View::default()
        }));
        {
            let tx = tx.clone();
            platform::set_player_hook(move |what| {
                let _ = tx.send(Cmd::System(what));
            });
        }
        let mut thread = Thread {
            player,
            view: view.clone(),
            wake: Box::new(wake),
            data: places.data,
            hidden: false,
            paused_since: None,
            resting: false,
            told: platform::Playback::default(),
            told_at: Instant::now(),
            saved_at: Instant::now(),
            saved: None,
            focus_asked: false,
            was_wanted: false,
        };
        thread.restore();
        let _ = std::thread::Builder::new()
            .name("amp-player".into())
            .spawn(move || thread.run(rx, wakes));
        Handle {
            tx,
            view,
            transport,
            shared,
            seen_track: 0,
        }
    }

    fn view(&self) -> std::sync::MutexGuard<'_, View> {
        self.view.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    /// A sender of commands (for a hook on another thread).
    pub fn sender(&self) -> Sender<Cmd> {
        self.tx.clone()
    }

    pub fn shared(&self) -> Arc<Shared> {
        self.view()
            .shared
            .clone()
            .unwrap_or_else(|| self.shared.clone())
    }

    pub fn playing(&self) -> bool {
        self.shared()
            .playing
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn position(&self) -> f64 {
        self.shared().position()
    }

    /// Where the track is, as shown (a seek just asked for until the sound
    /// has caught up with it).
    pub fn shown_position(&self) -> f64 {
        let (pending, asked) = {
            let view = self.view();
            (view.seek_pending, view.seek_asked)
        };
        if let Some(at) = pending {
            return at;
        }
        let heard = self.position();
        match asked {
            Some((at, when))
                if (heard - at).abs() > 0.5 && when.elapsed() < Duration::from_secs(1) =>
            {
                at
            }
            _ => heard,
        }
    }

    pub fn has_tracks(&self) -> bool {
        self.view().tracks > 0
    }

    pub fn tracks(&self) -> usize {
        self.view().tracks
    }

    pub fn ready(&self) -> bool {
        self.view().ready
    }

    pub fn seekable(&self) -> bool {
        self.view().seekable
    }

    pub fn duration(&self) -> Option<f64> {
        self.view().duration
    }

    pub fn title(&self) -> String {
        self.view().title.clone()
    }

    pub fn byline(&self) -> String {
        self.view().byline.clone()
    }

    pub fn status(&self) -> String {
        self.view().status.clone()
    }

    pub fn art(&self) -> Option<Arc<Art>> {
        self.view().art.clone()
    }

    pub fn current_path(&self) -> Option<String> {
        self.view().current_path.clone()
    }

    /// Whether another track is heard since the last call.
    pub fn pump(&mut self) -> Changed {
        let track = self.view().track;
        let changed = Changed {
            track: track != self.seen_track,
        };
        self.seen_track = track;
        changed
    }

    pub fn set_picked(&self, tracks: Vec<Picked>, play: bool) {
        self.send(Cmd::SetPicked(tracks, play));
    }

    pub fn add_picked(&self, tracks: Vec<Picked>) {
        self.send(Cmd::AddPicked(tracks));
    }

    pub fn set_tracks(&self, tracks: Vec<Track>, play: bool) {
        self.send(Cmd::SetTracks(tracks, play));
    }

    pub fn toggle(&self) -> bool {
        if !self.has_tracks() {
            return false;
        }
        self.send(Cmd::Toggle);
        true
    }

    pub fn pause(&self) {
        self.send(Cmd::Pause);
    }

    pub fn next(&self) {
        self.send(Cmd::Next);
    }

    pub fn previous(&self) {
        self.send(Cmd::Previous);
    }

    pub fn skip(&self, secs: f64) {
        self.send(Cmd::Skip(secs));
    }

    pub fn wind_to(&self, secs: f64) {
        self.send(Cmd::WindTo(secs));
    }

    pub fn shown(&self, shown: bool) {
        self.send(Cmd::Shown(shown));
    }
}

struct Thread {
    player: Player,
    view: Arc<Mutex<View>>,
    wake: Box<dyn Fn() + Send + Sync>,
    data: Option<PathBuf>,
    hidden: bool,
    /// Since when nothing plays (the engine rests after a while, hidden).
    paused_since: Option<Instant>,
    resting: bool,
    /// What the system was told last, and when.
    told: platform::Playback,
    told_at: Instant,
    saved_at: Instant,
    saved: Option<Saved>,
    /// The audio focus was asked for this playing.
    focus_asked: bool,
    /// Whether music was wanted at the last look (a change is saved).
    was_wanted: bool,
}

impl Thread {
    /// The state kept from the last time: the list, the track where it
    /// was, paused.
    fn restore(&mut self) {
        let Some(saved) = self.data.as_deref().and_then(Saved::load) else {
            return;
        };
        if saved.tracks.is_empty() {
            return;
        }
        platform::log_line(&format!(
            "restored: {} tracks, track {} at {:.0} s",
            saved.tracks.len(),
            saved.index + 1,
            saved.position
        ));
        self.player
            .restore(saved.tracks.clone(), saved.index, saved.position);
        self.saved = Some(saved);
    }

    fn run(mut self, rx: Receiver<Cmd>, wakes: Receiver<()>) {
        loop {
            let wait = self.wait();
            crossbeam_channel::select! {
                recv(rx) -> cmd => match cmd {
                    Ok(cmd) => self.command(cmd),
                    Err(_) => return,
                },
                recv(wakes) -> _ => {}
                default(wait) => {}
            }
            for cmd in rx.try_iter() {
                self.command(cmd);
            }
            for _ in wakes.try_iter() {}
            crate::watch::doing("amp-player", "the player's news");
            self.player.pump();
            self.after();
            crate::watch::done("amp-player");
        }
    }

    /// How long to wait for work: while music plays, until the state is
    /// to be saved; while a retry waits, a second; else as long as nothing
    /// comes (no timer wakes an idle app).
    fn wait(&self) -> Duration {
        if self.player.retry_pending() {
            Duration::from_secs(1)
        } else if self.player.transport.wants_playing() {
            SAVE_EVERY
        } else if self.hidden && !self.resting && self.paused_since.is_some() {
            ENGINE_REST
        } else {
            Duration::from_secs(3600)
        }
    }

    fn command(&mut self, cmd: Cmd) {
        // (a command after the engine rested: a new one first - playing
        // at once if that is what the command asks)
        if self.resting && !matches!(cmd, Cmd::Shown(_)) {
            let play = matches!(
                cmd,
                Cmd::Play
                    | Cmd::Toggle
                    | Cmd::System(ToPlayer::Media(
                        platform::MEDIA_PLAY | platform::MEDIA_TOGGLE,
                        _
                    ))
            );
            self.resting = false;
            self.player.new_engine(play);
            platform::log_line("a new engine after the rest");
            if play {
                return;
            }
        }
        let p = &mut self.player;
        match cmd {
            Cmd::SetPicked(tracks, play) => p.set_picked(tracks, play),
            Cmd::AddPicked(tracks) => {
                let play = p.transport.wants_playing();
                p.add_picked(tracks, play);
            }
            Cmd::SetTracks(tracks, play) => p.set_tracks(tracks, play),
            Cmd::Toggle => {
                p.toggle();
            }
            Cmd::Play => p.play(),
            Cmd::Pause => p.pause(),
            Cmd::Next => p.next(false),
            Cmd::Previous => p.previous(),
            Cmd::Skip(secs) => p.skip(secs),
            Cmd::WindTo(secs) => p.wind_to(secs),
            Cmd::Shown(shown) => {
                self.hidden = !shown;
                // (hidden: what was heard is kept now - the process may go)
                if !shown {
                    self.save(true);
                }
            }
            Cmd::System(ToPlayer::Media(command, arg)) => {
                platform::log_line(&format!("media: {}", media_name(command)));
                match command {
                    platform::MEDIA_PLAY => p.play(),
                    platform::MEDIA_PAUSE | platform::MEDIA_STOP => p.pause(),
                    platform::MEDIA_TOGGLE => {
                        p.toggle();
                    }
                    platform::MEDIA_NEXT => p.next(false),
                    platform::MEDIA_PREVIOUS => p.previous(),
                    platform::MEDIA_FORWARD => p.skip(15.0),
                    platform::MEDIA_REWIND => p.skip(-15.0),
                    platform::MEDIA_SEEK_TO => p.wind_to(arg.max(0) as f64 / 1000.0),
                    _ => {}
                }
            }
            Cmd::System(ToPlayer::Focus(granted)) => {
                if !granted && p.transport.wants_playing() {
                    p.pause();
                    p.status = "Another app is using the sound.".into();
                }
            }
        }
    }

    /// After the news: the audio focus, the rest, what the system and the
    /// screen are told, the state kept.
    fn after(&mut self) {
        let wanted = self.player.transport.wants_playing();
        // (the focus is asked for when music is to play; given up on a
        // pause)
        if wanted && !self.focus_asked {
            self.focus_asked = true;
            platform::ask_audio_focus();
        } else if !wanted && self.focus_asked {
            self.focus_asked = false;
            platform::abandon_audio_focus();
        }
        // (hidden and paused a while: the engine rests)
        if wanted {
            self.paused_since = None;
        } else if self.paused_since.is_none() {
            self.paused_since = Some(Instant::now());
        }
        if self.hidden
            && !self.resting
            && self
                .paused_since
                .is_some_and(|since| since.elapsed() >= ENGINE_REST)
        {
            self.save(true);
            self.player.rest_engine();
            self.resting = true;
            platform::log_line("hidden and paused: the engine rests");
        }
        self.tell();
        self.update_view();
        // (played or paused: kept at once - a force-stop right after finds
        // it)
        let changed = wanted != self.was_wanted;
        self.was_wanted = wanted;
        self.save(changed);
    }

    /// The system told what plays, when that changed (and now and then
    /// while it plays, the position again).
    fn tell(&mut self) {
        let p = &self.player;
        let state = if !p.has_tracks() {
            0
        } else if p.transport.wants_playing() {
            3
        } else if p.status == "End of the list" {
            1
        } else {
            2
        };
        // (as shown: a seek just asked for is where the track is, before
        // the sound has caught up with it; a track not open yet starts
        // where it is asked to, not where the last one was)
        let position = if self.player.ready() {
            self.player.shown_position()
        } else {
            self.player.seek_pending().unwrap_or(0.0)
        };
        let p = &self.player;
        let now = platform::Playback {
            state,
            position_ms: (position.max(0.0) * 1000.0) as u64,
            duration_ms: (p.duration.unwrap_or(0.0).max(0.0) * 1000.0) as u64,
            title: p.title(),
            artist: p.info.artist.clone(),
            album: p.info.album.clone(),
        };
        let changed = now.state != self.told.state
            || now.title != self.told.title
            || now.artist != self.told.artist
            || now.duration_ms != self.told.duration_ms
            // (a seek, or the position drifted from what the system works
            // out by itself from the last one)
            || now.position_ms.abs_diff(self.expected_position()) > 2000;
        if changed {
            if crate::watch::names() && state == 3 {
                platform::log_line(&format!(
                    "playing: {:.0} s into {}",
                    now.position_ms as f64 / 1000.0,
                    now.title
                ));
            }
            platform::playback(&now);
            self.told = now;
            self.told_at = Instant::now();
        } else if state == 3 && self.told_at.elapsed() >= SAVE_EVERY {
            if crate::watch::names() {
                platform::log_line(&format!(
                    "playing: {:.0} s into {}",
                    now.position_ms as f64 / 1000.0,
                    now.title
                ));
            }
            self.told_at = Instant::now();
        }
    }

    /// Where the system takes the track to be now, from what it was told.
    fn expected_position(&self) -> u64 {
        if self.told.state == 3 {
            self.told.position_ms + self.told_at.elapsed().as_millis() as u64
        } else {
            self.told.position_ms
        }
    }

    fn update_view(&self) {
        let p = &self.player;
        let (seek_pending, seek_asked) = p.seeks();
        let mut view = self.view.lock().unwrap_or_else(|e| e.into_inner());
        let track_changed =
            view.title != p.title() || view.current_path.as_deref() != p.current_path();
        view.title = p.title();
        view.byline = p.byline();
        view.status = p.status.clone();
        view.duration = p.duration;
        view.ready = p.ready();
        view.seekable = p.info.seekable;
        view.tracks = p.playlist.tracks.len();
        view.art = p.info.art.clone();
        view.current_path = p.current_path().map(str::to_owned);
        if track_changed {
            view.track += 1;
        }
        view.seek_pending = seek_pending;
        view.seek_asked = seek_asked;
        view.shared = Some(p.shared().clone());
        drop(view);
        (self.wake)();
    }

    /// The state kept: when it changed (another list or track), and while
    /// music plays every [`SAVE_EVERY`] (`now`: at once).
    fn save(&mut self, now: bool) {
        let Some(dir) = self.data.clone() else {
            return;
        };
        let p = &self.player;
        let state = Saved {
            tracks: p.picked().to_vec(),
            index: p.current_index().unwrap_or(0),
            position: p.position(),
        };
        let same_list = self
            .saved
            .as_ref()
            .is_some_and(|s| s.tracks == state.tracks && s.index == state.index);
        let due = p.transport.wants_playing() && self.saved_at.elapsed() >= SAVE_EVERY;
        if same_list && !due && !now {
            return;
        }
        // (no pick yet: nothing to keep - an older state stays)
        if state.tracks.is_empty() {
            return;
        }
        if let Err(e) = state.save(&dir) {
            log::warn!(target: "axrossamp", "the state could not be saved: {e}");
        }
        self.saved_at = Instant::now();
        self.saved = Some(state);
    }
}

fn media_name(command: i32) -> &'static str {
    match command {
        platform::MEDIA_PLAY => "play",
        platform::MEDIA_PAUSE => "pause",
        platform::MEDIA_TOGGLE => "play/pause",
        platform::MEDIA_NEXT => "next",
        platform::MEDIA_PREVIOUS => "previous",
        platform::MEDIA_STOP => "stop",
        platform::MEDIA_FORWARD => "fast forward",
        platform::MEDIA_REWIND => "rewind",
        platform::MEDIA_SEEK_TO => "seek",
        _ => "unknown",
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::SystemTime;

    fn settled(path: &std::path::Path, bytes: &[u8]) {
        let mut file = std::fs::File::create(path).unwrap();
        file.write_all(bytes).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(600))
            .unwrap();
    }

    /// A WAV of `secs` seconds of a tone (8 kHz, mono, 16 bits).
    fn wav(secs: u32) -> Vec<u8> {
        let samples = 8000 * secs;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + samples * 2).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&8000u32.to_le_bytes());
        out.extend_from_slice(&16000u32.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(samples * 2).to_le_bytes());
        for i in 0..samples {
            let v = (8000.0 * (i as f32 * 440.0 * std::f32::consts::TAU / 8000.0).sin()) as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    fn none(_: &str) -> Option<i32> {
        None
    }

    fn until(what: &str, check: impl Fn() -> bool) {
        let begun = Instant::now();
        while !check() {
            assert!(
                begun.elapsed() < Duration::from_secs(10),
                "{what}: not within 10 s"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn the_player_comes_back_where_it_was_and_answers_the_systems_commands() {
        let _one_at_a_time = platform::TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("axrossamp-service-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tone = wav(10);
        let (one, two) = (dir.join("1 one.wav"), dir.join("2 two.wav"));
        settled(&one, &tone);
        settled(&two, &tone);
        let picked = |path: &std::path::Path, name: &str| Picked {
            uri: path.to_string_lossy().into_owned(),
            folder: String::new(),
            name: name.to_owned(),
            size: None,
        };
        // (what an earlier run kept: the second track, 2.5 s in)
        Saved {
            tracks: vec![picked(&one, "1 one.wav"), picked(&two, "2 two.wav")],
            index: 1,
            position: 2.5,
        }
        .save(&dir)
        .unwrap();

        let handle = Handle::start(
            Places {
                cache: Some(dir.join("open")),
                data: Some(dir.clone()),
            },
            none,
            true,
            || {},
        );
        until("restored", || handle.tracks() == 2 && handle.ready());
        assert_eq!(handle.title(), "2 two");
        assert!(!handle.transport.wants_playing());

        // (the screen's play, then the system's commands - as the media
        // session's, the notification's, a headset's arrive)
        assert!(handle.toggle());
        until("playing", || handle.transport.wants_playing());
        platform::dispatch(platform::EVENT_MEDIA, platform::MEDIA_PAUSE, 0, 0, 0, "");
        until("paused by the media session", || {
            !handle.transport.wants_playing()
        });
        platform::dispatch(
            platform::EVENT_MEDIA,
            platform::MEDIA_SEEK_TO,
            6000,
            0,
            0,
            "",
        );
        until("sought", || (handle.shown_position() - 6.0).abs() < 0.3);
        platform::dispatch(platform::EVENT_MEDIA, platform::MEDIA_REWIND, 0, 0, 0, "");
        until("15 s back: the start", || handle.shown_position() < 0.3);
        platform::dispatch(platform::EVENT_MEDIA, platform::MEDIA_PREVIOUS, 0, 0, 0, "");
        until("the track before", || handle.title() == "1 one");
        platform::dispatch(platform::EVENT_MEDIA, platform::MEDIA_NEXT, 0, 0, 0, "");
        until("the next track", || handle.title() == "2 two");

        // (hidden: kept at once, the list and the track)
        handle.shown(false);
        until("saved", || Saved::load(&dir).is_some_and(|s| s.index == 1));
        platform::clear_player_hook();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
