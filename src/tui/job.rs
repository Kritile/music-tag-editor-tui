//! Bounded worker pool and identity for TUI background work.

use super::message::Message;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct JobId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum JobKind {
    Scan,
    Check,
    HashDuplicates,
    MusicBrainzSearch,
    MusicBrainzRelease,
    ExportPreview,
    Export,
    RenamePreview,
    Rename,
    Apply,
    Quarantine,
    Recovery,
    History,
}

pub(super) enum JobEvent {
    Started(JobId),
    Update(JobId, Box<Message>),
    Completed(JobId, Box<Message>),
    Failed(JobId, JobError),
    Cancelled(JobId),
}

#[derive(Debug, thiserror::Error)]
pub(super) enum JobError {
    #[error("background job panicked")]
    Panicked,
    #[error("background job queue is full")]
    QueueFull,
    #[error("background workers could not start or have stopped")]
    WorkersUnavailable,
    #[error("job ID exhausted")]
    IdExhausted,
}

struct ActiveJob {
    id: JobId,
    kind: JobKind,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
pub(super) struct JobManager {
    next_id: u64,
    active: Option<ActiveJob>,
}

pub(super) struct JobContext {
    id: JobId,
    cancelled: Arc<AtomicBool>,
    sender: Sender<Message>,
}

impl JobContext {
    pub(super) fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub(super) fn progress(&self, message: Message) {
        let _ = self
            .sender
            .send(Message::Job(JobEvent::Update(self.id, Box::new(message))));
    }
}

type Work = Box<dyn FnOnce() + Send + 'static>;

fn pool() -> Result<&'static SyncSender<Work>, JobError> {
    static POOL: OnceLock<Result<SyncSender<Work>, ()>> = OnceLock::new();
    POOL.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel::<Work>(32);
        let receiver = Arc::new(Mutex::new(receiver));
        for index in 0..2 {
            let receiver = Arc::clone(&receiver);
            if thread::Builder::new()
                .name(format!("music-tui-worker-{index}"))
                .spawn(move || {
                    loop {
                        let next = receiver
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .recv();
                        match next {
                            Ok(work) => work(),
                            Err(_) => break,
                        }
                    }
                })
                .is_err()
            {
                return Err(());
            }
        }
        Ok(sender)
    })
    .as_ref()
    .map_err(|_| JobError::WorkersUnavailable)
}

impl JobManager {
    pub(super) fn is_busy(&self) -> bool {
        self.active.is_some()
    }

    pub(super) fn active_kind(&self) -> Option<JobKind> {
        self.active.as_ref().map(|job| job.kind)
    }

    pub(super) fn is_active(&self, id: JobId) -> bool {
        self.active.as_ref().is_some_and(|job| job.id == id)
    }

    pub(super) fn complete(&mut self, id: JobId) -> bool {
        if self.is_active(id) {
            self.active = None;
            true
        } else {
            false
        }
    }

    pub(super) fn cancel(&mut self, abandon: bool) -> bool {
        let Some(active) = &self.active else {
            return false;
        };
        active.cancelled.store(true, Ordering::Relaxed);
        if abandon {
            self.active = None;
        }
        true
    }

    pub(super) fn launch(
        &mut self,
        kind: JobKind,
        sender: Sender<Message>,
        work: impl FnOnce(JobContext) -> Message + Send + 'static,
    ) -> Result<JobId, JobError> {
        let id = JobId(self.next_id.checked_add(1).ok_or(JobError::IdExhausted)?);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let notify = sender.clone();
        let task: Work = Box::new(move || {
            if worker_cancelled.load(Ordering::Relaxed) {
                let _ = notify.send(Message::Job(JobEvent::Cancelled(id)));
                return;
            }
            let _ = notify.send(Message::Job(JobEvent::Started(id)));
            let context = JobContext {
                id,
                cancelled: worker_cancelled,
                sender: notify.clone(),
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(context)));
            let event = match result {
                Ok(message) => JobEvent::Completed(id, Box::new(message)),
                Err(_) => JobEvent::Failed(id, JobError::Panicked),
            };
            let _ = notify.send(Message::Job(event));
        });
        match pool()?.try_send(task) {
            Ok(()) => {
                self.cancel(true);
                self.next_id = id.0;
                self.active = Some(ActiveJob {
                    id,
                    kind,
                    cancelled,
                });
                Ok(id)
            }
            Err(TrySendError::Full(_)) => Err(JobError::QueueFull),
            Err(TrySendError::Disconnected(_)) => Err(JobError::WorkersUnavailable),
        }
    }
}

#[cfg(test)]
impl JobManager {
    pub(super) fn set_busy_for_test(&mut self, busy: bool) {
        self.active = busy.then(|| ActiveJob {
            id: JobId(0),
            kind: JobKind::Scan,
            cancelled: Arc::new(AtomicBool::new(false)),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_job_cancels_old_identity() {
        let (sender, _receiver) = mpsc::channel();
        let mut jobs = JobManager::default();
        let old = jobs
            .launch(JobKind::Scan, sender.clone(), |_| {
                Message::Job(JobEvent::Failed(JobId(0), JobError::Panicked))
            })
            .unwrap();
        let new = jobs
            .launch(JobKind::HashDuplicates, sender, |_| {
                Message::Job(JobEvent::Failed(JobId(0), JobError::Panicked))
            })
            .unwrap();
        assert_ne!(old, new);
        assert!(!jobs.is_active(old));
        assert!(jobs.is_active(new));
        assert!(!jobs.complete(old));
        assert!(jobs.complete(new));
    }
}
