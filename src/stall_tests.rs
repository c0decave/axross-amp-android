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
    };
    let stuck = Picked {
        uri: "content://stuck/3".into(),
        folder: String::new(),
        name: "3 three.mp3".into(),
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
