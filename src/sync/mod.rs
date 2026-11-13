//! The sync engine: a bounded pool of workers, each taking repositories
//! from a shared queue and reporting one `Outcome` per repository.
//!
//! The engine never touches the database or the terminal. It sends events
//! back to the calling thread, which is the only one that writes to SQLite
//! and to the terminal, so neither needs locking.

pub mod hooks;
pub mod worker;

use std::collections::VecDeque;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::Error;
use crate::error::exit;
use crate::model::{Outcome, Tally, VcsKind};
use crate::proc::{Captured, Tracer};
use crate::vcs;
pub use worker::Job;

/// What the engine reports, in the order it happened.
#[derive(Debug, Clone)]
pub enum Event {
    /// A worker took this repository.
    Started { repo_id: i64 },
    /// A clone started; clones can run for minutes.
    Cloning { repo_id: i64 },
    /// A repository is done.
    Finished {
        repo_id: i64,
        outcome: Outcome,
        duration: Duration,
    },
    /// A git command finished. Only sent when `Options::trace` is set.
    Trace(Captured),
    /// Time passed. Sent every `Options::tick` while repositories are
    /// syncing, so live progress can show how long each has taken.
    Tick,
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Upper bound on concurrent repositories; capped at the job count.
    pub workers: usize,
    /// Fetch and report how far behind, without changing working trees.
    pub fetch_only: bool,
    /// Send `Event::Trace` for every git command (`--verbose`).
    pub trace: bool,
    /// Send `Event::Tick` this often. `None` sends none.
    pub tick: Option<Duration>,
}

enum Message {
    Event(Event),
    WorkerDone,
}

/// Sync `jobs` with up to `options.workers` threads, calling `on_event` on
/// the calling thread for each event. Once `stop` is set, workers take no
/// new jobs, so repositories not yet started produce no events.
pub fn run(jobs: Vec<Job>, options: &Options, stop: &AtomicBool, mut on_event: impl FnMut(Event)) {
    if jobs.is_empty() {
        return;
    }
    let workers = options.workers.clamp(1, jobs.len());
    let queue = Mutex::new(VecDeque::from(jobs));
    let (tx, rx) = mpsc::channel::<Message>();

    let tracer: Option<Tracer> = options.trace.then(|| {
        let tx = tx.clone();
        Arc::new(move |captured: &Captured| {
            let _ = tx.send(Message::Event(Event::Trace(captured.clone())));
        }) as Tracer
    });
    // One backend per kind; v1 has only Git.
    let backend = vcs::backend(VcsKind::Git, tracer);
    let backend = backend.as_ref();

    thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let queue = &queue;
            scope.spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let Some(job) = queue.lock().unwrap().pop_front() else {
                        break;
                    };
                    let repo_id = job.repo.id;
                    let send = |event| {
                        let _ = tx.send(Message::Event(event));
                    };
                    send(Event::Started { repo_id });
                    let started = Instant::now();
                    let outcome = match job.repo.vcs {
                        VcsKind::Git => {
                            worker::sync_one(&job, backend, options.fetch_only, stop, &mut |e| {
                                send(e)
                            })
                        }
                    };
                    send(Event::Finished {
                        repo_id,
                        outcome,
                        duration: started.elapsed(),
                    });
                }
                let _ = tx.send(Message::WorkerDone);
            });
        }
        // The tracer holds a sender too, so the channel never closes on its
        // own; count finished workers instead.
        let mut done = 0;
        let mut next_tick = options.tick.map(|tick| Instant::now() + tick);
        loop {
            let message = match next_tick {
                Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                    Ok(message) => Some(message),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match rx.recv() {
                    Ok(message) => Some(message),
                    Err(_) => break,
                },
            };
            // Ticks keep their pace even while other events stream in.
            if let (Some(at), Some(tick)) = (next_tick, options.tick)
                && Instant::now() >= at
            {
                on_event(Event::Tick);
                next_tick = Some(Instant::now() + tick);
            }
            match message {
                None => {}
                Some(Message::Event(event)) => on_event(event),
                Some(Message::WorkerDone) => {
                    done += 1;
                    if done == workers {
                        break;
                    }
                }
            }
        }
    });
}

/// The exit code for a finished run (docs/ARCHITECTURE.md section 10).
pub fn exit_code(tally: &Tally, strict: bool, interrupted: bool) -> u8 {
    if interrupted {
        exit::INTERRUPTED
    } else if tally.failed > 0 || tally.hooks_failed > 0 || (strict && tally.skipped > 0) {
        exit::FAILED
    } else {
        exit::OK
    }
}

/// An exclusive lock on `vuta.lock`, held until dropped.
#[derive(Debug)]
pub struct SyncLock {
    _file: File,
}

/// Take the sync lock at `path`, failing at once with `Error::Locked` if
/// another process holds it.
pub fn acquire_lock(path: &Path) -> Result<SyncLock, Error> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|err| anyhow::anyhow!("cannot open lock file {}: {err}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(SyncLock { _file: file }),
        Err(TryLockError::WouldBlock) => Err(Error::Locked),
        Err(TryLockError::Error(err)) => {
            Err(anyhow::anyhow!("cannot lock {}: {err}", path.display()).into())
        }
    }
}

/// The process-wide Ctrl-C flag. The first call installs a handler that
/// sets it (on SIGINT, and SIGTERM on Unix); later calls return the same
/// flag.
pub fn interrupt_flag() -> Result<&'static AtomicBool, Error> {
    static FLAG: AtomicBool = AtomicBool::new(false);
    static INSTALLED: OnceLock<Result<(), String>> = OnceLock::new();
    INSTALLED
        .get_or_init(|| {
            ctrlc::set_handler(|| FLAG.store(true, Ordering::Relaxed)).map_err(|e| e.to_string())
        })
        .clone()
        .map_err(|e| anyhow::anyhow!("cannot install the Ctrl-C handler: {e}"))?;
    Ok(&FLAG)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::model::{OutcomeKind, Repo};
    use crate::testutil::Fixture;

    fn jobs(fixture: &Fixture, n: i64) -> Vec<Job> {
        (1..=n)
            .map(|id| Job {
                repo: Repo {
                    id,
                    name: format!("r{id}"),
                    root_id: 1,
                    rel_path: format!("work/r{id}"),
                    vcs: VcsKind::Git,
                    url: fixture.url(),
                    branch: None,
                    clone_filter: None,
                    clone_depth: None,
                    enabled: true,
                },
                path: fixture.path("work").join(format!("r{id}")),
            })
            .collect()
    }

    fn options(workers: usize, trace: bool) -> Options {
        Options {
            workers,
            fetch_only: false,
            trace,
            tick: None,
        }
    }

    #[test]
    fn every_job_finishes_once() {
        let fixture = Fixture::new();
        let mut events = Vec::new();
        run(
            jobs(&fixture, 5),
            &options(3, false),
            &AtomicBool::new(false),
            |e| events.push(e),
        );

        let mut started = HashMap::new();
        let mut finished: HashMap<i64, OutcomeKind> = HashMap::new();
        for (i, event) in events.iter().enumerate() {
            match event {
                Event::Started { repo_id } => {
                    started.insert(*repo_id, i);
                }
                Event::Finished {
                    repo_id, outcome, ..
                } => {
                    assert!(started[repo_id] < i, "finished before started");
                    assert!(finished.insert(*repo_id, outcome.kind()).is_none());
                }
                Event::Cloning { .. } => {}
                Event::Trace(_) => panic!("trace without trace option"),
                Event::Tick => panic!("tick without tick option"),
            }
        }
        assert_eq!(finished.len(), 5);
        assert!(finished.values().all(|k| *k == OutcomeKind::Cloned));
    }

    #[test]
    fn traces_are_sent_when_asked() {
        let fixture = Fixture::new();
        let mut commands = Vec::new();
        run(
            jobs(&fixture, 1),
            &options(4, true),
            &AtomicBool::new(false),
            |e| {
                if let Event::Trace(captured) = e {
                    commands.push(captured.command);
                }
            },
        );
        assert!(
            commands.iter().any(|c| c.contains(" clone ")),
            "{commands:?}"
        );
    }

    #[test]
    fn a_set_stop_flag_starts_nothing() {
        let fixture = Fixture::new();
        let mut events = 0;
        run(
            jobs(&fixture, 3),
            &options(2, false),
            &AtomicBool::new(true),
            |_| events += 1,
        );
        assert_eq!(events, 0);
        assert!(!fixture.path("work").exists());
    }

    #[test]
    fn ticks_are_sent_when_asked() {
        let fixture = Fixture::new();
        let mut ticks = 0;
        let mut finished = 0;
        let options = Options {
            tick: Some(Duration::from_millis(1)),
            ..options(2, false)
        };
        run(
            jobs(&fixture, 2),
            &options,
            &AtomicBool::new(false),
            |e| match e {
                Event::Tick => ticks += 1,
                Event::Finished { .. } => finished += 1,
                _ => {}
            },
        );
        // Two clones take far longer than a millisecond.
        assert!(ticks > 0);
        assert_eq!(finished, 2);
    }

    #[test]
    fn no_jobs_is_fine() {
        run(
            Vec::new(),
            &options(4, false),
            &AtomicBool::new(false),
            |_| panic!("no events"),
        );
    }

    #[test]
    fn exit_codes() {
        let clean = Tally {
            updated: 2,
            skipped: 1,
            ..Default::default()
        };
        assert_eq!(exit_code(&clean, false, false), exit::OK);
        assert_eq!(exit_code(&clean, true, false), exit::FAILED);
        assert_eq!(
            exit_code(&Tally { failed: 1, ..clean }, false, false),
            exit::FAILED
        );
        assert_eq!(
            exit_code(
                &Tally {
                    hooks_failed: 1,
                    ..clean
                },
                false,
                false
            ),
            exit::FAILED
        );
        assert_eq!(exit_code(&clean, false, true), exit::INTERRUPTED);
    }

    #[test]
    fn lock_is_exclusive_until_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path: PathBuf = dir.path().join("vuta.lock");
        let held = acquire_lock(&path).unwrap();
        assert!(matches!(acquire_lock(&path), Err(Error::Locked)));
        drop(held);
        acquire_lock(&path).unwrap();
    }
}
