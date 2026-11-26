//! The player behind the screen: the desktop's engine and playlist, driven
//! for a list of picked tracks. No window and no Android in here - the
//! screen ([`crate::screen`]) and, in step 2, the playback service and its
//! MediaSession call the same few methods.

use crate::amp::engine::{Command, Engine, Event, TrackInfo};
use crate::amp::playlist::{self, Playlist};
use crate::amp::state::Track;
use crate::opener::Failure;
use crate::picked::{self, Picked};
use crate::source::{Queue, Reply, Request, Source};
use crate::transport::{PastEnd, SeekTo, Transport, seek_to};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Tracks in a row that may fail before the player stops trying the next
/// (a folder of files it cannot read does not spin through all of them).
const FAILURES_IN_A_ROW: u32 = 8;

/// A track found still being copied is tried again after this, then after
/// twice as long each time it has not grown ([`copying_wait`]), up to
/// [`COPYING_WAIT_MOST`]; after [`COPYING_ROUNDS`] looks without growth the
/// copy is taken to be stalled - a failure.
pub const COPYING_RETRY: Duration = Duration::from_secs(10);
pub const COPYING_WAIT_MOST: Duration = Duration::from_secs(120);
pub const COPYING_ROUNDS: u32 = 6;

/// How long a track that was still being copied `rounds` times without
/// growing waits before it is tried again.
pub fn copying_wait(rounds: u32, base: Duration) -> Duration {
    base.saturating_mul(1u32 << rounds.min(16))
        .min(COPYING_WAIT_MOST.max(base))
}

/// Whether the engine's word on its output means there is no sound (a
/// device gone for good, Bluetooth that keeps dropping: the desktop's
/// engine then gives up and waits to be told another output).
pub fn output_lost(said: &str) -> bool {
    said.starts_with("no sound")
}

/// How long the player waits before it asks the engine for the output
/// again, the `tries`-th time: 5 s, 10 s, 20 s, then every 30 s.
pub fn output_wait(tries: u32) -> Duration {
    Duration::from_secs((5u64 << tries.min(3)).min(30))
}

/// What changed while [`Player::pump`] read the engine's events.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    /// Another track is heard now (its tags and cover are in `info`).
    pub track: bool,
}

/// A track found still being copied.
#[derive(Clone, Copy, Debug)]
struct Copying {
    /// When it was found so last.
    since: Instant,
    /// Its size then.
    size: u64,
    /// The looks in a row it had not grown.
    rounds: u32,
}

pub struct Player {
    engine: Engine,
    headless: bool,
    wake: Arc<dyn Fn() + Send + Sync>,
    pub transport: Arc<Transport>,
    source: Source,
    pub playlist: Playlist,
    /// The generation of the track asked for last, and the one the engine
    /// has loaded.
    generation: u64,
    generations: u64,
    loaded: Option<u64>,
    /// The track named to follow: its index, URI and generation.
    queued: Option<(usize, String, u64)>,
    /// Where the tracks opened lately can be read (by generation).
    paths: HashMap<u64, String>,
    pub info: TrackInfo,
    pub duration: Option<f64>,
    /// A line to show: why a track did not play, the end of the list.
    pub status: String,
    failures: u32,
    /// The current track did not open (it failed, or was still being
    /// copied) and nothing else plays: Play opens it again.
    stalled: bool,
    /// The pick so far (a folder's tracks come in batches), and what the
    /// provider says each document holds.
    picked: Vec<Picked>,
    sizes: HashMap<String, u64>,
    /// Tracks found still being copied.
    copying: HashMap<String, Copying>,
    copying_base: Duration,
    /// Every track left was still being copied while music was wanted: the
    /// player tries the first of them again then - and plays it only if no
    /// pause came between (the transport's count).
    retry_at: Option<Instant>,
    retry_pauses: u64,
    /// The track opening now is such a retry: if it is still being copied,
    /// the next round waits - no other track is opened in this one.
    in_retry: bool,
    /// A seek asked for while the track was still opening (done when it is
    /// open, as asked: a skip or a wind), and the last seek asked for
    /// (shown until the sound is there).
    seek_pending: Option<(f64, PastEnd)>,
    seek_asked: Option<(f64, Instant)>,
    /// No sound output: when the engine is asked for it again, and how
    /// often it was.
    output_retry: Option<Instant>,
    output_tries: u32,
}

impl Player {
    /// `cache`: a folder of the app's own for the documents being played
    /// (None on the host: plain paths only). `wake` asks for a frame.
    pub fn start(
        cache: Option<PathBuf>,
        open_fd: fn(&str) -> Option<i32>,
        headless: bool,
        wake: impl Fn() + Send + Sync + Clone + 'static,
    ) -> Player {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let engine = new_engine(headless, &wake);
        let transport = Arc::new(Transport::new(engine.tx.clone()));
        let source = {
            let wake = wake.clone();
            Source::start(transport.clone(), cache, open_fd, move || wake())
        };
        Player {
            engine,
            headless,
            wake,
            transport,
            source,
            playlist: Playlist::default(),
            generation: 0,
            generations: 0,
            loaded: None,
            queued: None,
            paths: HashMap::new(),
            info: TrackInfo::default(),
            duration: None,
            status: String::new(),
            failures: 0,
            stalled: false,
            picked: Vec::new(),
            sizes: HashMap::new(),
            copying: HashMap::new(),
            copying_base: COPYING_RETRY,
            retry_at: None,
            retry_pauses: 0,
            in_retry: false,
            seek_pending: None,
            seek_asked: None,
            output_retry: None,
            output_tries: 0,
        }
    }

    /// A new engine, after the old one ended while the app rested hidden
    /// ([`crate::rest`]): the current track is opened again where it was,
    /// paused.
    pub fn new_engine(&mut self) {
        let at = self.position();
        self.engine = new_engine(self.headless, &self.wake);
        self.transport.set_engine(self.engine.tx.clone());
        self.loaded = None;
        self.queued = None;
        if let Some(index) = self.playlist.current.filter(|_| !self.stalled) {
            self.play_index(index, (at.max(0.0) * 1000.0) as u64, false);
        }
    }

    /// What the visualizations read: the samples played, the position.
    pub fn shared(&self) -> &Arc<crate::amp::engine::Shared> {
        &self.engine.shared
    }

    /// Whether sound is going out now.
    pub fn playing(&self) -> bool {
        self.engine.shared.playing.load(Ordering::Relaxed)
    }

    /// Seconds into the track.
    pub fn position(&self) -> f64 {
        self.engine.shared.position()
    }

    pub fn has_tracks(&self) -> bool {
        !self.playlist.tracks.is_empty()
    }

    /// The current track is loaded (its tags are known, it can be sought in).
    pub fn ready(&self) -> bool {
        self.loaded == Some(self.generation) && self.playlist.current.is_some()
    }

    /// Where the current track can be read from, once it is open.
    pub fn current_path(&self) -> Option<&str> {
        self.paths.get(&self.generation).map(String::as_str)
    }

    /// The title to show: the tag's, else the file's name.
    pub fn title(&self) -> String {
        if !self.info.title.is_empty() {
            return self.info.title.clone();
        }
        self.playlist
            .current
            .and_then(|i| self.playlist.tracks.get(i))
            .map(|t| {
                if t.title.is_empty() {
                    playlist::default_title(&t.path)
                } else {
                    t.title.clone()
                }
            })
            .unwrap_or_default()
    }

    /// The artist, and the album after it where both are known.
    pub fn byline(&self) -> String {
        match (self.info.artist.is_empty(), self.info.album.is_empty()) {
            (false, false) => format!("{} · {}", self.info.artist, self.info.album),
            (false, true) => self.info.artist.clone(),
            (true, false) => self.info.album.clone(),
            (true, true) => String::new(),
        }
    }

    /// The picked tracks as the list to play (in a folder's order), the
    /// first of them playing if `play`.
    pub fn set_picked(&mut self, tracks: Vec<Picked>, play: bool) {
        self.picked = tracks;
        let tracks = self.picked_tracks();
        self.set_tracks(tracks, play);
    }

    /// More of the same pick (a folder is read in batches): into the list
    /// in their places, the track playing goes on.
    pub fn add_picked(&mut self, more: Vec<Picked>, play: bool) {
        if self.picked.is_empty() || self.playlist.current.is_none() {
            let mut all = std::mem::take(&mut self.picked);
            all.extend(more);
            return self.set_picked(all, play);
        }
        self.picked.extend(more);
        let current = self.current_uri();
        let tracks = self.picked_tracks();
        let index = current.and_then(|uri| tracks.iter().position(|t| t.path == uri));
        let shuffle = self.playlist.shuffle;
        let repeat = self.playlist.repeat;
        self.playlist = Playlist::default();
        self.playlist.shuffle = shuffle;
        self.playlist.repeat = repeat;
        self.playlist.add_tracks(tracks);
        if let Some(index) = index {
            self.playlist.play(index);
        }
    }

    fn picked_tracks(&mut self) -> Vec<Track> {
        picked::sort(&mut self.picked, playlist::natural_cmp);
        self.sizes = self
            .picked
            .iter()
            .filter_map(|p| Some((p.uri.clone(), p.size?)))
            .collect();
        self.picked
            .iter()
            .map(|p| Track {
                title: picked::title(&p.name),
                path: p.uri.clone(),
                ..Track::default()
            })
            .collect()
    }

    pub fn set_tracks(&mut self, tracks: Vec<Track>, play: bool) {
        self.playlist = Playlist::default();
        self.playlist.add_tracks(tracks);
        self.queued = None;
        self.failures = 0;
        self.copying.clear();
        self.retry_at = None;
        self.status.clear();
        if self.playlist.tracks.is_empty() {
            self.stop();
        } else {
            self.play_index(0, 0, play);
        }
    }

    fn new_generation(&mut self) -> u64 {
        self.generations += 1;
        self.generations
    }

    /// Opens track `index` at `start_ms`, playing or not.
    pub fn play_index(&mut self, index: usize, start_ms: u64, play: bool) {
        let Some(track) = self.playlist.tracks.get(index).cloned() else {
            return;
        };
        self.generation = self.new_generation();
        self.queued = None;
        self.stalled = false;
        self.in_retry = false;
        self.seek_pending = None;
        self.seek_asked = None;
        self.playlist.play(index);
        self.info = TrackInfo::default();
        self.duration = (track.duration_ms > 0).then(|| track.duration_ms as f64 / 1000.0);
        // (what is wanted, said now: a pause while the document is still
        // being opened counts)
        self.transport.intend(play);
        self.source.send(Request::Load {
            name: file_name(&track),
            size: self.sizes.get(&track.path).copied(),
            uri: track.path,
            start_ms,
            play,
            generation: self.generation,
        });
    }

    /// Play, or pause what plays. False: there is nothing to play.
    pub fn toggle(&mut self) -> bool {
        let Some(current) = self.playlist.current else {
            return false;
        };
        if self.transport.wants_playing() {
            self.pause();
        } else if self.stalled {
            // (it did not open before: once more, now)
            self.copying.clear();
            self.play_index(current, 0, true);
        } else {
            self.transport.play();
        }
        true
    }

    /// Play (the audio focus came; or the user's play).
    pub fn play(&mut self) {
        if !self.transport.wants_playing() {
            self.toggle();
        }
    }

    pub fn pause(&mut self) {
        self.retry_at = None;
        self.transport.pause();
    }

    pub fn stop(&mut self) {
        self.retry_at = None;
        self.transport.stop();
    }

    /// To `secs` into the track: from 0:00 at the least, past its end on to
    /// the next track (as at its end; paused, it waits paused). Asked while
    /// the track is still opening, it is done once it is open.
    pub fn seek(&mut self, secs: f64) {
        self.seek_as(secs, PastEnd::Next);
    }

    /// A wind (a long press, the seek bar) to `secs`: as [`Player::seek`],
    /// but held a moment before the end instead of ending the track.
    pub fn wind_to(&mut self, secs: f64) {
        self.seek_as(secs, PastEnd::Stay);
    }

    /// `secs` back (negative) or on from where the track is.
    pub fn skip(&mut self, secs: f64) {
        let from = match self.seek_pending {
            Some((at, _)) => at,
            None => self.shown_position(),
        };
        self.seek(from + secs);
    }

    fn seek_as(&mut self, secs: f64, past_end: PastEnd) {
        if self.playlist.current.is_none() {
            return;
        }
        let opening = !self.ready();
        let duration = if opening { None } else { self.duration };
        match seek_to(secs, duration, past_end) {
            SeekTo::Nowhere => {}
            SeekTo::At(at) if opening => self.seek_pending = Some((at, past_end)),
            SeekTo::At(at) => {
                if self.info.seekable {
                    self.transport.seek(at);
                    self.seek_asked = Some((at, Instant::now()));
                }
            }
            SeekTo::Next => {
                let play = self.transport.wants_playing();
                match self.next_playable(true) {
                    Some(i) => self.play_index(i, 0, play),
                    None => {
                        self.stop();
                        self.status = "End of the list".into();
                    }
                }
            }
        }
    }

    /// Where the track is, as shown: a seek just asked for (or one waiting
    /// for the track to open) until the sound has caught up with it, else
    /// the sound's.
    pub fn shown_position(&mut self) -> f64 {
        if let Some((at, _)) = self.seek_pending {
            return at;
        }
        let heard = self.position();
        match self.seek_asked {
            Some((at, when))
                if (heard - at).abs() > 0.5 && when.elapsed() < Duration::from_secs(1) =>
            {
                at
            }
            _ => {
                self.seek_asked = None;
                heard
            }
        }
    }

    /// The seek asked for while the track was opening (for the tests).
    pub fn seek_pending(&self) -> Option<f64> {
        self.seek_pending.map(|(at, _)| at)
    }

    /// Whether the player waits to try a track still being copied again
    /// (for the tests).
    pub fn retrying(&self) -> bool {
        self.retry_at.is_some()
    }

    /// The first wait before a track still being copied is tried again
    /// (the tests shorten it).
    pub fn set_copying_wait(&mut self, base: Duration) {
        self.copying_base = base;
    }

    /// Whether `track` was found still being copied too short a while ago
    /// to try it again.
    fn waits(&self, track: &Track) -> bool {
        self.copying
            .get(&track.path)
            .is_some_and(|c| c.since.elapsed() < copying_wait(c.rounds, self.copying_base))
    }

    /// The track after the current one that is worth trying: not one found
    /// still being copied a moment ago.
    fn next_playable(&mut self, ended: bool) -> Option<usize> {
        let here = self.playlist.current;
        for _ in 0..self.playlist.tracks.len().max(1) {
            let i = self.playlist.next(ended)?;
            let waits = self.playlist.tracks.get(i).is_some_and(|t| self.waits(t));
            if !waits {
                return Some(i);
            }
            // (passed over: the list goes on from it)
            self.playlist.play(i);
        }
        if let Some(here) = here {
            self.playlist.play(here);
        }
        None
    }

    /// The next track: at the end of one (`ended`) or because it was asked,
    /// playing if music is wanted (a track that ended while a pause came
    /// does not start the next).
    pub fn next(&mut self, ended: bool) {
        let play = self.transport.wants_playing();
        match self.next_playable(ended) {
            Some(i) => self.play_index(i, 0, play),
            None => {
                self.stop();
                self.status = "End of the list".into();
            }
        }
    }

    /// The track before - or, a few seconds into this one, its start.
    pub fn previous(&mut self) {
        if self.ready() && self.info.seekable && self.position() > 3.0 {
            self.seek(0.0);
        } else if let Some(i) = self.playlist.previous() {
            let play = self.transport.wants_playing();
            self.play_index(i, 0, play);
        }
    }

    /// Reads what the engine and the opening thread said since the last
    /// call, and names the track that follows to the engine (gapless).
    pub fn pump(&mut self) -> Changed {
        let mut changed = Changed::default();
        while let Ok(reply) = self.source.replies.try_recv() {
            match reply {
                Reply::Opened { generation, path } => {
                    self.paths.insert(generation, path);
                    // (only the latest are of use: the current, the queued)
                    let oldest = self.generations.saturating_sub(8);
                    self.paths.retain(|g, _| *g >= oldest);
                }
                Reply::Failed {
                    generation,
                    failure: Failure::Copying(why, size),
                } if generation == self.generation => self.copying(why, size),
                Reply::Failed {
                    generation,
                    failure: Failure::Failed(message),
                } if generation == self.generation => self.failed(message),
                Reply::Failed { .. } => {}
            }
        }
        while let Ok(event) = self.engine.events.try_recv() {
            match event {
                Event::Loaded { generation, info } if generation == self.generation => {
                    self.take_info(info);
                    changed.track = true;
                }
                Event::Advanced { generation, info } => {
                    // (the queued track is heard: it is the current one now)
                    let Some((index, uri, queued)) = self.queued.take() else {
                        continue;
                    };
                    if queued != generation {
                        continue;
                    }
                    let index = if self
                        .playlist
                        .tracks
                        .get(index)
                        .is_some_and(|t| t.path == uri)
                    {
                        Some(index)
                    } else {
                        self.playlist.tracks.iter().position(|t| t.path == uri)
                    };
                    self.generation = generation;
                    if let Some(index) = index {
                        self.playlist.play(index);
                    }
                    self.duration = None;
                    self.take_info(info);
                    changed.track = true;
                }
                Event::Ended { generation } if generation == self.generation => self.next(true),
                Event::Failed {
                    generation,
                    message,
                } if generation == self.generation => self.failed(message),
                Event::Output(said) => self.output(said),
                _ => {}
            }
        }
        // every track left was still being copied: one of them once more -
        // the one whose wait ends first - playing only if no pause came
        // meanwhile
        if self.retry_at.is_some_and(|at| Instant::now() >= at) {
            self.retry_at = None;
            if self.transport.pauses() == self.retry_pauses
                && let Some((i, _)) = self.next_retry()
            {
                self.play_index(i, 0, true);
                self.in_retry = true;
            }
        }
        // no sound output: the engine is asked for it again, now and then
        if self.output_retry.is_some_and(|at| Instant::now() >= at) {
            self.output_retry = None;
            platform_log(&format!(
                "asking for the sound output again ({})",
                self.output_tries
            ));
            self.transport.command(Command::Output(None));
        }
        self.sync_queue();
        changed
    }

    /// The engine's word on its output.
    fn output(&mut self, said: String) {
        log::info!(target: "axrossamp", "output: {said}");
        if output_lost(&said) {
            // (the desktop's engine says to choose one in its Mixer tab - a
            // tab this app has not; it is tried again here by itself)
            let wait = output_wait(self.output_tries);
            self.output_tries += 1;
            self.output_retry = Some(Instant::now() + wait);
            self.status = format!("No sound output - trying again in {} s", wait.as_secs());
        } else if self.output_tries > 0 {
            self.output_tries = 0;
            self.output_retry = None;
            if self.status.starts_with("No sound output") {
                self.status.clear();
            }
        }
    }

    fn take_info(&mut self, info: TrackInfo) {
        self.loaded = Some(self.generation);
        self.failures = 0;
        self.stalled = false;
        self.status.clear();
        self.duration = info.duration.or(self.duration);
        self.info = info;
        if let Some(uri) = self.current_uri() {
            self.copying.remove(&uri);
        }
        // (a seek asked for while it was opening, as it was asked)
        if let Some((at, past_end)) = self.seek_pending.take() {
            self.seek_as(at, past_end);
        }
    }

    fn current_uri(&self) -> Option<String> {
        self.playlist
            .current
            .and_then(|i| self.playlist.tracks.get(i))
            .map(|t| t.path.clone())
    }

    /// The current track is still being copied (`size` bytes so far):
    /// said, passed over for now (it is not a failure), tried again later -
    /// unless it has not grown for [`COPYING_ROUNDS`] looks.
    fn copying(&mut self, why: String, size: u64) {
        let name = self.title();
        let Some(uri) = self.current_uri() else {
            return;
        };
        let rounds = match self.copying.get(&uri) {
            Some(seen) if seen.size == size => seen.rounds + 1,
            _ => 0,
        };
        if rounds >= COPYING_ROUNDS {
            self.copying.remove(&uri);
            return self.failed(format!("{why} - and the copy does not go on"));
        }
        self.copying.insert(
            uri,
            Copying {
                since: Instant::now(),
                size,
                rounds,
            },
        );
        log::info!(target: "axrossamp", "a track is {why}: passed over for now");
        let play = self.transport.wants_playing();
        // (a retry's round opens no other track)
        let next = if std::mem::take(&mut self.in_retry) {
            None
        } else {
            self.next_playable(true)
        };
        match next {
            Some(i) => {
                self.play_index(i, 0, play);
                self.status = format!("{name}: {why} - tried again later");
            }
            None => {
                let now = Instant::now();
                let due = self.next_retry().map_or(now, |(_, due)| due.max(now));
                self.stop();
                self.stalled = true;
                self.status = format!(
                    "{name}: {why} - trying again in {} s",
                    due.duration_since(now).as_secs().max(1)
                );
                if play {
                    self.retry_at = Some(due);
                    self.retry_pauses = self.transport.pauses();
                }
            }
        }
    }

    /// The track still being copied to try first, and when.
    fn next_retry(&self) -> Option<(usize, Instant)> {
        self.playlist
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let c = self.copying.get(&t.path)?;
                Some((i, c.since + copying_wait(c.rounds, self.copying_base)))
            })
            .min_by_key(|(_, due)| *due)
    }

    /// The current track did not open or play: said, and on to the next -
    /// unless the ones before it failed as well.
    fn failed(&mut self, message: String) {
        let name = self.title();
        self.failures += 1;
        self.status = format!("{name}: {message}");
        log::warn!(target: "axrossamp", "a track failed: {message}");
        let more = self.failures < FAILURES_IN_A_ROW.min(self.playlist.tracks.len() as u32);
        match self.next_playable(true) {
            Some(i) if more => {
                let play = self.transport.wants_playing();
                self.play_index(i, 0, play);
                // (play_index starts a new track: the line about this one stays)
                self.status = format!("{name}: {message}");
            }
            _ => {
                self.stop();
                self.stalled = true;
            }
        }
    }

    /// Names the track that follows the current one to the engine whenever
    /// that changes: it is opened and decoded on behind this one, no gap.
    /// (The same track at another place in the list - a folder's batches
    /// move it - is no change.)
    fn sync_queue(&mut self) {
        let want = if self.loaded == Some(self.generation) {
            self.playlist
                .peek_next()
                .and_then(|i| Some((i, self.playlist.tracks.get(i)?.clone())))
        } else {
            None
        };
        match (&mut self.queued, &want) {
            (Some((i, uri, _)), Some((wi, track))) if *uri == track.path => {
                *i = *wi;
                return;
            }
            (None, None) => return,
            _ => {}
        }
        match want {
            Some((index, track)) => {
                let generation = self.new_generation();
                self.queued = Some((index, track.path.clone(), generation));
                self.source.send(Request::Queue(Some(Queue {
                    uri: track.path.clone(),
                    name: file_name(&track),
                    size: self.sizes.get(&track.path).copied(),
                    generation,
                })));
            }
            None => {
                self.queued = None;
                self.source.send(Request::Queue(None));
            }
        }
    }

    /// The engine itself, for what has no method here yet (tests; step 2).
    pub fn send(&self, command: Command) {
        self.engine.send(command);
    }
}

fn new_engine(headless: bool, wake: &Arc<dyn Fn() + Send + Sync>) -> Engine {
    let wake = wake.clone();
    let wake = move || wake();
    if headless {
        Engine::start_headless(wake)
    } else {
        Engine::start(wake)
    }
}

fn platform_log(text: &str) {
    crate::platform::log_line(text);
}

/// A name with an ending for the opened document: the decoder's hint. The
/// track's title lost its ending ([`picked::title`]); the URI's last part
/// usually still has it.
fn file_name(track: &Track) -> String {
    let last = track
        .path
        .rsplit(['/', ':'])
        .next()
        .unwrap_or_default()
        .rsplit("%2F")
        .next()
        .unwrap_or_default();
    if picked::extension(last).is_some() {
        last.to_owned()
    } else {
        track.title.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<i32> {
        None
    }

    fn track(path: &str) -> Track {
        Track {
            path: path.to_owned(),
            ..Track::default()
        }
    }

    #[test]
    fn the_file_name_keeps_the_ending_for_the_decoder() {
        // (a tree document's URI: the path percent-encoded in its last part)
        let t = Track {
            path: "content://com.android.externalstorage.documents/tree/primary%3AMusic/document/primary%3AMusic%2FAlbum%2F01%20One.flac".into(),
            title: "01 One".into(),
            ..Track::default()
        };
        assert_eq!(file_name(&t), "01%20One.flac");
        // (a media document has only a number: the title is all there is)
        let t = Track {
            path: "content://com.android.providers.media.documents/document/audio%3A1234".into(),
            title: "Two".into(),
            ..Track::default()
        };
        assert_eq!(file_name(&t), "Two");
        assert_eq!(file_name(&track("/music/three.ogg")), "three.ogg");
    }

    #[test]
    fn an_empty_player_has_nothing_to_play() {
        let mut player = Player::start(None, none, true, || {});
        assert!(!player.has_tracks());
        assert!(!player.toggle());
        assert!(!player.playing());
        assert!(!player.ready());
        assert_eq!(player.title(), "");
        player.next(false);
        assert_eq!(player.status, "End of the list");
        player.previous();
        assert_eq!(player.pump(), Changed::default());
    }

    #[test]
    fn picked_tracks_are_played_in_a_folders_order() {
        let mut player = Player::start(None, none, true, || {});
        let picked = picked::parse("u10\tA\t10 Ten.mp3\nu2\tA\t2 Two.mp3\nu1\t\t1 One.mp3\n");
        player.set_picked(picked, false);
        let uris: Vec<&str> = player
            .playlist
            .tracks
            .iter()
            .map(|t| t.path.as_str())
            .collect();
        assert_eq!(uris, ["u1", "u2", "u10"]);
        assert_eq!(player.playlist.current, Some(0));
        // (the title until the tags are read: the name without its ending)
        assert_eq!(player.title(), "1 One");
        assert!(!player.transport.wants_playing());
    }

    #[test]
    fn tracks_that_cannot_be_opened_are_told_and_the_player_stops() {
        let mut player = Player::start(None, none, true, || {});
        // (documents, and nothing here can open one)
        player.set_tracks(
            vec![track("content://none/a.mp3"), track("content://none/b.mp3")],
            true,
        );
        let begun = std::time::Instant::now();
        while player.transport.wants_playing() {
            assert!(
                begun.elapsed().as_secs() < 20,
                "still trying: {}",
                player.status
            );
            player.pump();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!player.status.is_empty());
        assert!(!player.playing());
    }
}
