//! From what a picker names to what the engine plays: each track about to
//! play is opened ([`crate::opener`]) off the window's thread and handed to
//! the engine from there, in the order it was asked for.
//!
//! Opening a document waits on the system, and a busy system (a phone
//! short of storage, its media scanner going through files being copied
//! in) may take seconds or not answer at all. So nothing here waits on one
//! opening alone: the opening runs on a worker of its own, a newer request
//! is taken while it runs - a track to load sets aside the opening of the
//! one queued behind, or of a track asked for before - and an opening that
//! takes longer than its deadline is given up on.

use crate::amp::engine::Queued;
use crate::opener::{Begun, Failure, Opener, Opening};
use crate::transport::Transport;
use crate::watch;
use crossbeam_channel::{Receiver, Sender};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

pub enum Request {
    /// Open the track and have the engine load it.
    Load {
        uri: String,
        name: String,
        /// What the provider says the document holds (None: unknown).
        size: Option<u64>,
        start_ms: u64,
        play: bool,
        generation: u64,
    },
    /// Open the track that follows and name it to the engine (None: none).
    Queue(Option<Queue>),
}

/// The track to follow.
pub struct Queue {
    pub uri: String,
    pub name: String,
    pub size: Option<u64>,
    pub generation: u64,
}

#[derive(Debug, PartialEq)]
pub enum Reply {
    /// The track of `generation` is open under `path`.
    Opened { generation: u64, path: String },
    /// It could not be opened (or is still being copied).
    Failed { generation: u64, failure: Failure },
}

pub struct Source {
    tx: Sender<Request>,
    pub replies: Receiver<Reply>,
}

/// The thread's name in the log and the watchdog's report.
const THREAD: &str = "amp-source";

#[derive(Clone)]
enum Kind {
    Load { start_ms: u64, play: bool },
    Queue,
}

/// What the thread is opening.
struct Job {
    kind: Kind,
    uri: String,
    name: String,
    size: Option<u64>,
    generation: u64,
}

impl Job {
    fn what(&self) -> String {
        format!(
            "opening {} ({}, track {})",
            watch::name(&self.name),
            match self.kind {
                Kind::Load { .. } => "to play",
                Kind::Queue => "to follow",
            },
            self.generation
        )
    }
}

/// What was asked for and not yet begun: only the last track to load is
/// opened (Next pressed five times opens one) and only what is to be
/// queued behind it.
#[derive(Default)]
struct Asked {
    load: Option<Job>,
    queue: Option<Option<Job>>,
}

impl Asked {
    fn take(&mut self, request: Request) {
        match request {
            Request::Load {
                uri,
                name,
                size,
                start_ms,
                play,
                generation,
            } => {
                self.load = Some(Job {
                    kind: Kind::Load { start_ms, play },
                    uri,
                    name,
                    size,
                    generation,
                });
                self.queue = None;
            }
            Request::Queue(queue) => {
                self.queue = Some(queue.map(|q| Job {
                    kind: Kind::Queue,
                    uri: q.uri,
                    name: q.name,
                    size: q.size,
                    generation: q.generation,
                }));
            }
        }
    }
}

struct Worker {
    opener: Opener,
    transport: Arc<Transport>,
    replies: Sender<Reply>,
    wake: Box<dyn Fn() + Send>,
    /// The documents the engine has now: the track loaded last, the one
    /// queued behind it (the opener lets go of neither).
    playing: Option<String>,
    queued: Option<String>,
}

impl Worker {
    /// The end of `job`: the engine told, the player answered.
    fn finish(&mut self, job: Job, opened: Result<String, Failure>, since: Instant) {
        watch::done(THREAD);
        let ms = since.elapsed().as_millis();
        match &opened {
            Ok(_) => crate::platform::log_line(&format!("{} done in {ms} ms", job.what())),
            Err(failure) => crate::platform::log_line(&format!(
                "{}: {} (after {ms} ms)",
                job.what(),
                failure.message()
            )),
        }
        let generation = job.generation;
        match &opened {
            Ok(_) if matches!(job.kind, Kind::Load { .. }) => self.playing = Some(job.uri.clone()),
            Ok(_) => self.queued = Some(job.uri.clone()),
            Err(_) => {}
        }
        match (job.kind, opened) {
            (Kind::Load { start_ms, play }, Ok(path)) => {
                self.transport
                    .load(path.clone(), start_ms, play, generation);
                let _ = self.replies.send(Reply::Opened { generation, path });
            }
            (Kind::Load { .. }, Err(failure)) => {
                let _ = self.replies.send(Reply::Failed {
                    generation,
                    failure,
                });
            }
            (Kind::Queue, Ok(path)) => {
                let _ = self.replies.send(Reply::Opened {
                    generation,
                    path: path.clone(),
                });
                self.transport.queue(Some(Queued { path, generation }));
            }
            // (one that cannot be opened is not queued: it is tried again
            // when its turn comes, and told then)
            (Kind::Queue, Err(_)) => self.transport.queue(None),
        }
        (self.wake)();
    }

    /// Begins `job`: done at once (a plain path, a document open already,
    /// one that cannot be), or being opened.
    fn begin(&mut self, job: Job) -> Option<(Job, Opening)> {
        watch::doing(THREAD, job.what());
        let since = Instant::now();
        match self.opener.begin(&job.uri, &job.name, job.size) {
            Begun::Ready(path) => {
                self.finish(job, Ok(path), since);
                None
            }
            Begun::Failed(failure) => {
                self.finish(job, Err(failure), since);
                None
            }
            Begun::Opening(opening) => Some((job, opening)),
        }
    }

    fn run(mut self, rx: Receiver<Request>) {
        let mut asked = Asked::default();
        let mut current: Option<(Job, Opening)> = None;
        loop {
            // what was asked for meanwhile
            if current.is_none() && asked.load.is_none() && asked.queue.is_none() {
                match rx.recv() {
                    Ok(request) => asked.take(request),
                    Err(_) => return,
                }
            }
            for request in rx.try_iter() {
                asked.take(request);
            }
            // a newer request sets aside what is being opened: a load any
            // opening, a queue the opening of an earlier queue
            if let Some((job, _)) = &current {
                let set_aside = match job.kind {
                    Kind::Load { .. } => asked.load.is_some(),
                    Kind::Queue => asked.load.is_some() || asked.queue.is_some(),
                };
                if set_aside && let Some((job, opening)) = current.take() {
                    watch::done(THREAD);
                    crate::platform::log_line(&format!(
                        "{}: set aside for a newer request after {} ms",
                        job.what(),
                        opening.started.elapsed().as_millis()
                    ));
                    opening.abandon();
                }
            }
            if current.is_none() {
                if let Some(job) = asked.load.take() {
                    current = self.begin(job);
                } else if let Some(queue) = asked.queue.take() {
                    match queue {
                        Some(job) => current = self.begin(job),
                        None => self.transport.queue(None),
                    }
                }
            }
            // the opening, or a new request, or its deadline
            let Some((_, opening)) = current.as_mut() else {
                continue;
            };
            let left = opening.deadline.saturating_duration_since(Instant::now());
            let mut done = None;
            crossbeam_channel::select! {
                recv(rx) -> request => match request {
                    Ok(request) => asked.take(request),
                    Err(_) => return,
                },
                recv(opening.said()) -> said => done = opening.handle(said),
                default(left) => {
                    done = Some(Err(Failure::Failed(crate::opener::too_slow())));
                }
            }
            if let Some(result) = done
                && let Some((job, opening)) = current.take()
            {
                let since = opening.started;
                let in_use: Vec<&str> = [self.playing.as_deref(), self.queued.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect();
                let opened = result.map(|placed| self.opener.finish(&job.uri, placed, &in_use));
                opening.abandon();
                self.finish(job, opened, since);
            }
        }
    }
}

impl Source {
    /// `dir`: where the opened tracks get their names (emptied first; None:
    /// only plain paths can be played). `open_fd` opens a content:// URI.
    /// `wake` is called when a reply waits.
    pub fn start(
        transport: Arc<Transport>,
        dir: Option<PathBuf>,
        open_fd: fn(&str) -> Option<i32>,
        wake: impl Fn() + Send + 'static,
    ) -> Source {
        let (tx, rx) = crossbeam_channel::unbounded::<Request>();
        let (replies_tx, replies) = crossbeam_channel::unbounded();
        let worker = Worker {
            opener: Opener::new(dir, open_fd),
            transport,
            replies: replies_tx,
            wake: Box::new(wake),
            playing: None,
            queued: None,
        };
        let _ = std::thread::Builder::new()
            .name(THREAD.into())
            .spawn(move || worker.run(rx));
        Source { tx, replies }
    }

    pub fn send(&self, request: Request) {
        let _ = self.tx.send(request);
    }
}
