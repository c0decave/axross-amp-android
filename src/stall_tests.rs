//! The folder hang's tests (a phone short of storage: music copied in
//! while it is picked, a provider that answers slowly or not at all):
//!
//! - the window's turns never wait on the opening of a document - a
//!   heartbeat over a player whose tracks are partly still being written
//!   and partly behind a provider that does not answer;
//! - the decoder ends, with an error or with what is there, on a file cut
//!   short and on one that grows while it is read.
//!
//! The fixtures (tests/media/sine.flac, sine.mp3: one second of 440 Hz,
//! from axross's tests/fixtures/audio, made by its generate.sh).

use crate::amp::engine;
use crate::picked::Picked;
use crate::player::Player;
use std::fs::File;
use std::io::Write;
use std::os::fd::IntoRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/media")
            .join(name),
    )
    .unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("axrossamp-stall-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A whole file, written a while ago.
fn settled(path: &Path, bytes: &[u8]) {
    let mut file = File::create(path).unwrap();
    file.write_all(bytes).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(600))
        .unwrap();
}

/// Stands in for the Java side: content://test<path> is the file at
/// <path>; content://stuck/... is a provider that does not answer for
/// three seconds, then cannot open it.
fn open_fd(uri: &str) -> Option<i32> {
    if uri.starts_with("content://stuck/") {
        std::thread::sleep(Duration::from_secs(3));
        return None;
    }
    let path = uri.strip_prefix("content://test")?;
    File::open(path).ok().map(IntoRawFd::into_raw_fd)
}

/// `f` decodes `path` to its end (or its error) within `limit`.
fn ends_within(limit: Duration, path: &Path) -> Result<(), String> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let file = path.to_string_lossy().into_owned();
    std::thread::spawn(move || {
        let probed = engine::probe(&file).map(|_| ());
        let decoded = engine::decode_file(&file, 48_000).map(|_| ());
        let _ = tx.send((probed, decoded));
    });
    rx.recv_timeout(limit)
        .map(|_| ())
        .map_err(|_| format!("{} did not end within {limit:?}", path.display()))
}

#[test]
fn the_decoder_ends_on_a_file_cut_short() {
    let dir = scratch("cut");
    for name in ["sine.mp3", "sine.flac"] {
        let whole = fixture(name);
        for share in [0usize, 1, 8, 2, 3] {
            // (none of it, one byte, an eighth, half, a third... of the file)
            let bytes = match share {
                0 => 0,
                1 => 1,
                n => whole.len() / n,
            };
            let path = dir.join(format!("{share}-{name}"));
            std::fs::write(&path, &whole[..bytes]).unwrap();
            ends_within(Duration::from_secs(10), &path).unwrap();
        }
        // (the whole one decodes, as a check of the fixture)
        let path = dir.join(name);
        std::fs::write(&path, &whole).unwrap();
        let (frames, _) = engine::decode_file(&path.to_string_lossy(), 48_000).unwrap();
        assert!(frames > 40_000, "{name}: {frames} frames");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_decoder_ends_on_a_file_that_grows_while_it_is_read() {
    let dir = scratch("grows");
    for name in ["sine.mp3", "sine.flac"] {
        let whole = fixture(name);
        let path = dir.join(name);
        let mut file = File::create(&path).unwrap();
        file.write_all(&whole[..whole.len() / 4]).unwrap();
        // (the rest comes in pieces while the decoder reads)
        let rest = whole[whole.len() / 4..].to_vec();
        let writer = std::thread::spawn(move || {
            for piece in rest.chunks(512) {
                file.write_all(piece).unwrap();
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        ends_within(Duration::from_secs(15), &path).unwrap();
        writer.join().unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// One turn of the window: the call, the news, the clock.
type Turn<'a> = dyn FnMut(&mut Player, &mut dyn FnMut(&mut Player)) + 'a;

#[test]
fn the_window_never_waits_on_a_busy_provider() {
    let dir = scratch("heartbeat");
    let music = dir.join("music");
    std::fs::create_dir_all(&music).unwrap();
    let mp3 = fixture("sine.mp3");
    let flac = fixture("sine.flac");
    settled(&music.join("1 one.mp3"), &mp3);
    settled(&music.join("4 four.flac"), &flac);
    settled(&music.join("5 five.mp3"), &mp3);
    // (being copied in: a quarter there, the rest coming)
    let growing = music.join("2 two.mp3");
    let mut copy = File::create(&growing).unwrap();
    copy.write_all(&mp3[..mp3.len() / 4]).unwrap();
    let rest = mp3[mp3.len() / 4..].to_vec();
    let copier = std::thread::spawn(move || {
        for piece in rest.chunks(256) {
            copy.write_all(piece).unwrap();
            std::thread::sleep(Duration::from_millis(40));
        }
    });
    let track = |name: &str| Picked {
        uri: format!("content://test{}", music.join(name).display()),
        folder: String::new(),
        name: name.to_owned(),
        size: None,
    };
    let stuck = Picked {
        uri: "content://stuck/3".into(),
        folder: String::new(),
        name: "3 three.mp3".into(),
        size: None,
    };

    let mut player = Player::start(Some(dir.join("open")), open_fd, true, || {});
    let mut slowest = Duration::ZERO;
    let mut turn = |player: &mut Player, f: &mut dyn FnMut(&mut Player)| {
        let begun = Instant::now();
        f(player);
        player.pump();
        let _ = player.shown_position();
        slowest = slowest.max(begun.elapsed());
    };
    let current = |player: &Player| {
        player
            .playlist
            .current
            .map(|i| player.playlist.tracks[i].title.clone())
    };
    let wait_for = |player: &mut Player,
                    turn: &mut Turn,
                    what: &str,
                    until: &dyn Fn(&Player) -> bool,
                    limit: Duration| {
        let begun = Instant::now();
        while !until(player) {
            assert!(
                begun.elapsed() < limit,
                "{what}: not within {limit:?} (now {:?}, status {:?})",
                current(player),
                player.status
            );
            turn(player, &mut |_| {});
            std::thread::sleep(Duration::from_millis(16));
        }
    };

    // the first batch of a folder (paused: a second of music would end)
    turn(&mut player, &mut |p| {
        p.set_picked(vec![track("1 one.mp3"), track("2 two.mp3")], false)
    });
    wait_for(
        &mut player,
        &mut turn,
        "the first track opens",
        &|p| p.ready(),
        Duration::from_secs(10),
    );
    assert_eq!(current(&player).as_deref(), Some("1 one"));
    // more of the folder, read meanwhile: the track playing stays
    turn(&mut player, &mut |p| {
        p.add_picked(
            vec![track("5 five.mp3"), stuck.clone(), track("4 four.flac")],
            false,
        )
    });
    assert_eq!(player.playlist.tracks.len(), 5);
    assert_eq!(current(&player).as_deref(), Some("1 one"));

    // on: the next is still being copied - passed over, the one after it
    // is behind a provider that does not answer
    turn(&mut player, &mut |p| p.next(false));
    wait_for(
        &mut player,
        &mut turn,
        "the copied track is passed over",
        &|p| p.status.contains("still being copied"),
        Duration::from_secs(5),
    );
    assert_eq!(current(&player).as_deref(), Some("3 three"));
    // on again at once: the stuck opening is set aside, the next one plays
    let asked = Instant::now();
    turn(&mut player, &mut |p| p.next(false));
    wait_for(
        &mut player,
        &mut turn,
        "the track after the stuck one opens",
        &|p| p.ready(),
        Duration::from_secs(2),
    );
    assert_eq!(current(&player).as_deref(), Some("4 four"));
    assert!(asked.elapsed() < Duration::from_secs(2));
    // seeking while it is open, and back past the start
    turn(&mut player, &mut |p| p.skip(-15.0));
    turn(&mut player, &mut |p| p.wind_to(0.5));

    copier.join().unwrap();
    assert!(
        slowest < Duration::from_millis(100),
        "a turn of the window took {slowest:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_seek_while_the_track_opens_waits_for_it() {
    let dir = scratch("pending");
    let mut player = Player::start(Some(dir.join("open")), open_fd, true, || {});
    player.set_picked(
        vec![Picked {
            uri: "content://stuck/1".into(),
            folder: String::new(),
            name: "1 one.mp3".into(),
            size: None,
        }],
        false,
    );
    assert!(!player.ready());
    player.skip(15.0);
    assert_eq!(player.seek_pending(), Some(15.0));
    player.skip(15.0);
    assert_eq!(player.seek_pending(), Some(30.0));
    // (before the start: the start; no length known yet: not held back)
    player.skip(-100.0);
    assert_eq!(player.seek_pending(), Some(0.0));
    player.seek(500.0);
    assert_eq!(player.seek_pending(), Some(500.0));
    assert_eq!(player.shown_position(), 500.0);
    player.seek(f64::NAN);
    assert_eq!(player.seek_pending(), Some(500.0));
    let _ = std::fs::remove_dir_all(&dir);
}

// -- the retries of tracks still being copied (the audit's M1, M2) ------------------------------

use std::collections::HashMap;
use std::sync::Mutex;

/// How often each document was handed over.
static OPENED: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);

fn counted_open_fd(uri: &str) -> Option<i32> {
    *OPENED
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .entry(uri.to_owned())
        .or_default() += 1;
    open_fd(uri)
}

fn opened(uri: &str) -> usize {
    OPENED
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(uri).copied())
        .unwrap_or(0)
}

/// A file being copied in: written to just now (and kept so by `touch`).
fn being_copied(path: &Path) {
    std::fs::write(path, b"ID3 a quarter of it").unwrap();
}

fn touch(path: &Path) {
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now())
        .unwrap();
}

fn pump_until(player: &mut Player, what: &str, until: impl Fn(&Player) -> bool) {
    let begun = Instant::now();
    while !until(player) {
        assert!(
            begun.elapsed() < Duration::from_secs(10),
            "{what}: not within 10 s (status {:?})",
            player.status
        );
        player.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn picked(path: &Path, name: &str) -> Picked {
    Picked {
        uri: format!("content://test{}", path.display()),
        folder: String::new(),
        name: name.to_owned(),
        size: None,
    }
}

#[test]
fn a_pause_while_a_copy_is_waited_for_stays_a_pause() {
    let dir = scratch("retry-pause");
    let track = dir.join("1 one.mp3");
    being_copied(&track);
    let mut player = Player::start(Some(dir.join("open")), counted_open_fd, true, || {});
    player.set_copying_wait(Duration::from_millis(100));
    player.set_picked(vec![picked(&track, "1 one.mp3")], true);
    pump_until(&mut player, "still being copied", |p| p.retrying());
    // (the user's pause: no retry, nothing starts by itself)
    player.pause();
    assert!(!player.retrying());
    // (play again: tried at once; then the system hides the app)
    touch(&track);
    assert!(player.toggle());
    pump_until(&mut player, "still being copied again", |p| p.retrying());
    let before = opened(&picked(&track, "").uri);
    player
        .transport
        .interrupt(crate::platform::Interrupt::Hidden);
    std::thread::sleep(Duration::from_millis(250));
    for _ in 0..20 {
        player.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!player.retrying());
    assert!(!player.transport.wants_playing());
    assert_eq!(
        opened(&picked(&track, "").uri),
        before,
        "opened again after a pause"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_retry_round_tries_one_track_and_a_stalled_copy_is_given_up() {
    let dir = scratch("retry-one");
    let one = dir.join("1 one.mp3");
    let two = dir.join("2 two.mp3");
    being_copied(&one);
    being_copied(&two);
    let mut player = Player::start(Some(dir.join("open")), counted_open_fd, true, || {});
    player.set_copying_wait(Duration::from_millis(400));
    player.set_picked(
        vec![picked(&one, "1 one.mp3"), picked(&two, "2 two.mp3")],
        true,
    );
    pump_until(&mut player, "both still being copied", |p| p.retrying());
    let (one_uri, two_uri) = (picked(&one, "").uri, picked(&two, "").uri);
    let (a, b) = (opened(&one_uri), opened(&two_uri));
    assert_eq!((a, b), (1, 1));
    // (a round: one of them only - the one that waited longest)
    pump_until(&mut player, "a retry", |_| opened(&one_uri) == 2);
    assert_eq!(opened(&two_uri), 1);
    std::thread::sleep(Duration::from_millis(100));
    player.pump();
    assert_eq!(opened(&two_uri), 1);
    // (the next round: the other one)
    pump_until(&mut player, "the other's retry", |_| opened(&two_uri) == 2);
    assert_eq!(opened(&one_uri), 2);
    player.set_copying_wait(Duration::from_millis(5));
    // (it does not grow: after a few rounds it is a failure, not a wait
    // for ever)
    pump_until(&mut player, "given up", |p| {
        p.status.contains("does not go on")
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_sound_output_is_asked_for_again_with_growing_waits() {
    use crate::player::{output_lost, output_wait};
    assert!(output_lost(
        "no sound: the output keeps failing - choose one in the Mixer tab"
    ));
    assert!(output_lost("no sound: no audio device"));
    assert!(!output_lost("AAudio (48000 Hz)"));
    assert!(!output_lost("no device (headless)"));
    let waits: Vec<u64> = (0..6).map(|t| output_wait(t).as_secs()).collect();
    assert_eq!(waits, [5, 10, 20, 30, 30, 30]);
}

#[test]
fn a_copy_is_waited_for_longer_each_time_it_has_not_grown() {
    use crate::player::{COPYING_RETRY, COPYING_WAIT_MOST, copying_wait};
    assert_eq!(copying_wait(0, COPYING_RETRY), COPYING_RETRY);
    assert_eq!(copying_wait(1, COPYING_RETRY), COPYING_RETRY * 2);
    assert_eq!(copying_wait(2, COPYING_RETRY), COPYING_RETRY * 4);
    assert_eq!(copying_wait(30, COPYING_RETRY), COPYING_WAIT_MOST);
}

// -- the decoders on files cut short and damaged (the audit's fuzz) ----------------------------

/// The decoders on every fixture cut at many places (a file still being
/// copied in) and with bytes overwritten: probe and decode end - with an
/// error or with what is there - and neither panics nor takes longer than
/// a few seconds. Run in the dev profile: overflow checks on.
#[test]
fn the_decoders_survive_files_cut_short_and_damaged() {
    let dir = scratch("fuzz");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixtures = [
        root.join("tests/media/sine.mp3"),
        root.join("tests/media/sine.flac"),
        root.join("axross-amp/tests/media/chirp.opus"),
        root.join("axross-amp/tests/media/capture.ogg"),
        root.join("axross-amp/tests/media/vbr-noxing-short.mp3"),
    ];
    let mut problems = Vec::new();
    let mut runs = 0;
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    for source in fixtures.iter().filter(|p| p.exists()) {
        let data = std::fs::read(source).unwrap();
        let ending = source.extension().unwrap().to_string_lossy().into_owned();
        let name = source.file_name().unwrap().to_string_lossy().into_owned();
        let len = data.len();
        let mut cuts: Vec<usize> = (0..16).chain([100, 400, 1000, 4096]).collect();
        cuts.extend((1..24).map(|k| len * k / 24));
        cuts.retain(|c| *c < len);
        cuts.sort_unstable();
        cuts.dedup();
        let mut cases: Vec<(String, Vec<u8>)> = cuts
            .into_iter()
            .map(|cut| (format!("{name} cut at {cut}/{len}"), data[..cut].to_vec()))
            .collect();
        for k in 0..16 {
            let mut damaged = data.clone();
            let at = if k < 6 { (k * 7) % len } else { len * k / 16 };
            let n = [1usize, 4, 16, 64][k % 4];
            for byte in damaged.iter_mut().skip(at).take(n) {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                *byte = seed as u8;
            }
            cases.push((format!("{name} damaged at {at} ({n} bytes)"), damaged));
        }
        for (i, (what, bytes)) in cases.into_iter().enumerate() {
            let path = dir.join(format!("{i}.{ending}"));
            std::fs::write(&path, &bytes).unwrap();
            runs += 1;
            let (tx, rx) = crossbeam_channel::bounded(1);
            let file = path.to_string_lossy().into_owned();
            std::thread::spawn(move || {
                let outcome = std::panic::catch_unwind(|| {
                    let _ = engine::probe(&file);
                    let _ = engine::decode_file(&file, 48_000);
                });
                let _ = tx.send(outcome.is_ok());
            });
            match rx.recv_timeout(Duration::from_secs(10)) {
                Ok(true) => {}
                Ok(false) => problems.push(format!("panic: {what}")),
                Err(_) => problems.push(format!("over 10 s: {what}")),
            }
        }
    }
    assert!(runs > 200, "only {runs} decodes");
    assert!(problems.is_empty(), "{runs} decodes: {problems:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}
