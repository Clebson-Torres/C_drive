use crate::models::{StorageMode, TransferPhase, TransferState, TransferStatus};
use chrono::{Duration as ChronoDuration, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::broadcast;

const COMPLETED_TTL_SECONDS: i64 = 300;

struct SpeedSample {
    instant: Instant,
    bytes_done: u64,
}

struct JobMeta {
    started_at: chrono::DateTime<Utc>,
}

struct HubState {
    cancelled: HashSet<String>,
    last_status: HashMap<String, TransferStatus>,
    speed_samples: HashMap<String, SpeedSample>,
    job_meta: HashMap<String, JobMeta>,
}

#[derive(Clone)]
pub struct ProgressHub {
    tx: broadcast::Sender<TransferStatus>,
    state: Arc<Mutex<HubState>>,
}

impl ProgressHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(8192);
        Self {
            tx,
            state: Arc::new(Mutex::new(HubState {
                cancelled: HashSet::new(),
                last_status: HashMap::new(),
                speed_samples: HashMap::new(),
                job_meta: HashMap::new(),
            })),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TransferStatus> {
        self.tx.subscribe()
    }

    pub fn snapshot(&self) -> Vec<TransferStatus> {
        let now = Utc::now();
        self.state
            .lock()
            .map(|s| {
                s.last_status
                    .values()
                    .filter(|status| {
                        !matches!(
                            status.state,
                            TransferState::Completed
                                | TransferState::Failed
                                | TransferState::Cancelled
                        ) || status.updated_at
                            >= now - ChronoDuration::seconds(COMPLETED_TTL_SECONDS)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn cancel(&self, job_id: &str) {
        if let Ok(mut s) = self.state.lock() {
            s.cancelled.insert(job_id.to_string());
        }
    }

    pub fn is_cancelled(&self, job_id: &str) -> bool {
        self.state
            .lock()
            .map(|s| s.cancelled.contains(job_id))
            .unwrap_or(true)
    }

    pub fn transition(
        &self,
        job_id: &str,
        file_name: &str,
        state: TransferState,
        phase: TransferPhase,
        storage_mode: Option<StorageMode>,
        done: u64,
        total: u64,
        err: Option<String>,
    ) {
        let now = Utc::now();
        let (started_at, speed_bps) = self.compute_timing(job_id, done, now, &state);
        let eta_seconds = if speed_bps > 0 && total > done {
            Some((total - done).div_ceil(speed_bps))
        } else {
            None
        };

        let status = TransferStatus {
            job_id: job_id.to_string(),
            file_name: file_name.to_string(),
            state: state.clone(),
            phase,
            storage_mode,
            bytes_done: done,
            bytes_total: total,
            error: err,
            speed_bps,
            eta_seconds,
            started_at,
            updated_at: now,
        };

        if let Ok(mut s) = self.state.lock() {
            s.last_status.insert(job_id.to_string(), status.clone());
            if matches!(
                state,
                TransferState::Completed | TransferState::Failed | TransferState::Cancelled
            ) {
                s.speed_samples.remove(job_id);
                s.cancelled.remove(job_id);
            }
        }
        let _ = self.tx.send(status);
    }

    fn compute_timing(
        &self,
        job_id: &str,
        bytes_done: u64,
        now: chrono::DateTime<Utc>,
        state: &TransferState,
    ) -> (chrono::DateTime<Utc>, u64) {
        let mut guard = match self.state.lock() {
            Ok(g) => g,
            Err(_) => return (now, 0),
        };
        let started_at = guard
            .job_meta
            .entry(job_id.to_string())
            .or_insert_with(|| JobMeta { started_at: now })
            .started_at;

        let speed_bps = if matches!(state, TransferState::Running) {
            let instant = Instant::now();
            let speed = if let Some(prev) = guard.speed_samples.get(job_id) {
                let elapsed = instant.duration_since(prev.instant).as_secs_f64();
                if elapsed > 0.05 && bytes_done >= prev.bytes_done {
                    ((bytes_done - prev.bytes_done) as f64 / elapsed) as u64
                } else {
                    0
                }
            } else {
                0
            };
            guard.speed_samples.insert(
                job_id.to_string(),
                SpeedSample {
                    instant,
                    bytes_done,
                },
            );
            speed
        } else {
            0
        };

        (started_at, speed_bps)
    }
}
