//! From what a picker names to what the engine plays: each track about to
//! play is opened ([`crate::opener`]) on a thread of its own - opening a
//! document, copying one most of all, never holds up a frame - and handed
//! to the engine from there, in the order it was asked for.

use crate::amp::engine::Queued;
use crate::opener::Opener;
use crate::transport::Transport;
use crossbeam_channel::{Receiver, Sender};
use std::path::PathBuf;
use std::sync::Arc;

pub enum Request {
    /// Open the track and have the engine load it.
    Load {
        uri: String,
        name: String,
        start_ms: u64,
        play: bool,
        generation: u64,
    },
    /// Open the track that follows and name it to the engine (None: none).
    Queue(Option<(String, String, u64)>),
}

#[derive(Debug, PartialEq)]
pub enum Reply {
    /// The track of `generation` is open under `path`.
    Opened { generation: u64, path: String },
    /// It could not be opened.
    Failed { generation: u64, message: String },
}

pub struct Source {
    tx: Sender<Request>,
    pub replies: Receiver<Reply>,
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
        let (rtx, replies) = crossbeam_channel::unbounded();
        let mut opener = Opener::new(dir, open_fd);
        let _ = std::thread::Builder::new()
            .name("amp-source".into())
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    // (what was asked for meanwhile: only the last track to
                    // load is opened - Next pressed five times opens one -
                    // and only what is to be queued behind it)
                    let mut load = None;
                    let mut queue = None;
                    for request in std::iter::once(first).chain(rx.try_iter()) {
                        match request {
                            Request::Load { .. } => {
                                load = Some(request);
                                queue = None;
                            }
                            Request::Queue(q) => queue = Some(q),
                        }
                    }
                    if let Some(Request::Load {
                        uri,
                        name,
                        start_ms,
                        play,
                        generation,
                    }) = load
                    {
                        let reply = match opener.open(&uri, &name) {
                            Ok(path) => {
                                transport.load(path.clone(), start_ms, play, generation);
                                Reply::Opened { generation, path }
                            }
                            Err(message) => Reply::Failed {
                                generation,
                                message,
                            },
                        };
                        let _ = rtx.send(reply);
                        wake();
                    }
                    match queue {
                        Some(Some((uri, name, generation))) => {
                            // (one that cannot be opened is not queued: it
                            // fails when its turn comes, and is told then)
                            let queued = opener.open(&uri, &name).ok().map(|path| {
                                let _ = rtx.send(Reply::Opened {
                                    generation,
                                    path: path.clone(),
                                });
                                Queued { path, generation }
                            });
                            transport.queue(queued);
                            wake();
                        }
                        Some(None) => transport.queue(None),
                        None => {}
                    }
                }
            });
        Source { tx, replies }
    }

    pub fn send(&self, request: Request) {
        let _ = self.tx.send(request);
    }
}
