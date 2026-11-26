//! The player behind the screen: the desktop's engine and playlist, driven
//! for a list of picked tracks. No window and no Android in here - the
//! screen ([`crate::screen`]) and, in step 2, the playback service and its
//! MediaSession call the same few methods.

use crate::amp::engine::{Command, Engine, Event, TrackInfo};
use crate::amp::playlist::{self, Playlist};
use crate::amp::state::Track;
use crate::opener::Failure;
use crate::picked::{self, Picked};
use crate::source::{Reply, Request, Source};
use crate::transport::{PastEnd, SeekTo, Transport, seek_to};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Tracks in a row that may fail before the player stops trying the next
/// (a folder of files it cannot read does not spin through all of them).
const FAILURES_IN_A_ROW: u32 = 8;

/// A track found still being copied is not tried again before this, and
/// when every track left was, the player tries again after it.
pub const COPYING_RETRY: Duration = Duration::from_secs(10);

/// What changed while [`Player::pump`] read the engine's events.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    /// Another track is heard now (its tags and cover are in `info`).
    pub track: bool,
}

pub struct Player {
    engine: Engine,
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
    /// The pick so far (a folder's tracks come in batches).
    picked: Vec<Picked>,
    /// Tracks found still being copied, and when.
    copying: HashMap<String, Instant>,
    /// Every track left was still being copied while music was wanted: the
    /// player tries again then.
    retry_at: Option<Instant>,
    /// A seek asked for while the track was still opening (done when it is
    /// open), and the last seek asked for (shown until the sound is there).
    seek_pending: Option<f64>,
    seek_asked: Option<(f64, Instant)>,
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
        let engine = if headless {
            Engine::start_headless(wake.clone())
        } else {
            Engine::start(wake.clone())
        };
        let transport = Arc::new(Transport::new(engine.tx.clone()));
        let source = Source::start(transport.clone(), cache, open_fd, wake);
        Player {
            engine,
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
            picked: Vec::new(),
            copying: HashMap::new(),
            retry_at: None,
            seek_pending: None,
            seek_asked: None,
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
        let current = self
            .playlist
            .current
            .and_then(|i| self.playlist.tracks.get(i))
            .map(|t| t.path.clone());
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
            uri: track.path,
            start_ms,
            play,
            generation: self.generation,
        });
    }

    /// Play, or pause what plays. False: there is nothing to play.
    pub fn toggle(&mut self) -> bool {
        if self.playlist.current.is_none() {
            return false;
        }
        if self.transport.wants_playing() {
            self.transport.pause();
        } else {
            self.transport.play();
        }
        true
    }

    pub fn pause(&mut self) {
        self.transport.pause();
    }

    pub fn stop(&mut self) {
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
        let from = self.seek_pending.unwrap_or_else(|| self.shown_position());
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
            SeekTo::At(at) if opening => self.seek_pending = Some(at),
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
        if let Some(at) = self.seek_pending {
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
        self.seek_pending
    }

    /// The track after the current one that is worth trying: one not found
    /// still being copied a moment ago.
    fn next_playable(&mut self, ended: bool) -> Option<usize> {
        let fresh = |copying: &HashMap<String, Instant>, track: &Track| {
            copying
                .get(&track.path)
                .is_some_and(|at| at.elapsed() < COPYING_RETRY)
        };
        let here = self.playlist.current;
        for _ in 0..self.playlist.tracks.len().max(1) {
            let i = self.playlist.next(ended)?;
            let skip = self
                .playlist
                .tracks
                .get(i)
                .is_some_and(|t| fresh(&self.copying, t));
            if !skip {
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

    /// The next track: at the end of one (`ended`) or because it was asked.
    pub fn next(&mut self, ended: bool) {
        let play = ended || self.transport.wants_playing();
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
                    failure: Failure::Copying(why),
                } if generation == self.generation => self.copying(why),
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
                Event::Output(name) => log::info!(target: "axrossamp", "output: {name}"),
                _ => {}
            }
        }
        // every track left was still being copied: once more, a while later
        if self.retry_at.is_some_and(|at| Instant::now() >= at) {
            self.retry_at = None;
            let first = self
                .playlist
                .tracks
                .iter()
                .position(|t| self.copying.contains_key(&t.path));
            if let Some(i) = first {
                self.copying.clear();
                self.play_index(i, 0, true);
            }
        }
        self.sync_queue();
        changed
    }

    fn take_info(&mut self, info: TrackInfo) {
        self.loaded = Some(self.generation);
        self.failures = 0;
        self.status.clear();
        self.duration = info.duration.or(self.duration);
        self.info = info;
        if let Some(uri) = self.current_uri() {
            self.copying.remove(&uri);
        }
        // (a seek asked for while it was opening)
        if let Some(at) = self.seek_pending.take() {
            self.seek(at);
        }
    }

    fn current_uri(&self) -> Option<String> {
        self.playlist
            .current
            .and_then(|i| self.playlist.tracks.get(i))
            .map(|t| t.path.clone())
    }

    /// The current track is still being copied: said, passed over for now
    /// (it is not a failure), tried again later.
    fn copying(&mut self, why: String) {
        let name = self.title();
        if let Some(uri) = self.current_uri() {
            self.copying.insert(uri, Instant::now());
        }
        log::info!(target: "axrossamp", "a track is {why}: passed over for now");
        let play = self.transport.wants_playing();
        match self.next_playable(true) {
            Some(i) => {
                self.play_index(i, 0, play);
                self.status = format!("{name}: {why} - tried again later");
            }
            None => {
                self.stop();
                self.status = format!(
                    "{name}: {why} - trying again in {} s",
                    COPYING_RETRY.as_secs()
                );
                if play {
                    self.retry_at = Some(Instant::now() + COPYING_RETRY);
                }
            }
        }
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
            _ => self.stop(),
        }
    }

    /// Names the track that follows the current one to the engine whenever
    /// that changes: it is opened and decoded on behind this one, no gap.
    fn sync_queue(&mut self) {
        let want = if self.loaded == Some(self.generation) {
            self.playlist
                .peek_next()
                .and_then(|i| Some((i, self.playlist.tracks.get(i)?.clone())))
        } else {
            None
        };
        let same = match (&self.queued, &want) {
            (Some((i, uri, _)), Some((wi, track))) => i == wi && *uri == track.path,
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        match want {
            Some((index, track)) => {
                let generation = self.new_generation();
                self.queued = Some((index, track.path.clone(), generation));
                self.source.send(Request::Queue(Some((
                    track.path.clone(),
                    file_name(&track),
                    generation,
                ))));
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
