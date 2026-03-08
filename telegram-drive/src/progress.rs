use crate::models::{StorageMode, TransferPhase, TransferState, TransferStatus};
use chrono::{Duration as ChronoDuration, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::broadcast;
use tokio::time::{sleep, Duration};

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
    paused: HashSet<String>,
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
                paused: HashSet::new(),
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

    pub fn has_active_transfers(&self) -> bool {
        self.state
            .lock()
            .map(|s| {
                s.last_status.values().any(|status| {
                    matches!(status.state, TransferState::Queued | TransferState::Running)
                })
            })
            .unwrap_or(false)
    }

    pub fn is_job_active(&self, job_id: &str) -> bool {
        self.state
            .lock()
            .map(|s| {
                s.last_status
                    .get(job_id)
                    .map(|status| {
                        matches!(
                            status.state,
                            TransferState::Queued | TransferState::Running | TransferState::Paused
                        )
                    })
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    pub fn cancel(&self, job_id: &str) {
        if let Ok(mut s) = self.state.lock() {
            s.cancelled.insert(job_id.to_string());
            s.paused.remove(job_id);
        }
    }

    pub fn is_cancelled(&self, job_id: &str) -> bool {
        self.state
            .lock()
            .map(|s| s.cancelled.contains(job_id))
            .unwrap_or(true)
    }

    pub fn pause(&self, job_id: &str) {
        let snapshot = if let Ok(mut s) = self.state.lock() {
            s.paused.insert(job_id.to_string());
            if let Some(status) = s.last_status.get_mut(job_id) {
                if !matches!(
                    status.state,
                    TransferState::Completed | TransferState::Failed | TransferState::Cancelled
                ) {
                    status.state = TransferState::Paused;
                    status.speed_bps = 0;
                    status.eta_seconds = None;
                    status.updated_at = Utc::now();
                    Some(status.clone())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        if let Some(status) = snapshot {
            let _ = self.tx.send(status);
        }
    }

    pub fn resume(&self, job_id: &str) {
        let snapshot = if let Ok(mut s) = self.state.lock() {
            s.paused.remove(job_id);
            if let Some(status) = s.last_status.get_mut(job_id) {
                if matches!(status.state, TransferState::Paused) {
                    status.state = TransferState::Running;
                    status.speed_bps = 0;
                    status.eta_seconds = None;
                    status.updated_at = Utc::now();
                    Some(status.clone())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        if let Some(status) = snapshot {
            let _ = self.tx.send(status);
        }
    }

    pub fn is_paused(&self, job_id: &str) -> bool {
        self.state
            .lock()
            .map(|s| s.paused.contains(job_id))
            .unwrap_or(false)
    }

    pub async fn wait_if_paused(&self, job_id: &str) {
        while self.is_paused(job_id) && !self.is_cancelled(job_id) {
            sleep(Duration::from_millis(200)).await;
        }
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
        let paused = self.is_paused(job_id);
        let effective_state = if paused
            && matches!(state, TransferState::Queued | TransferState::Running)
        {
            TransferState::Paused
        } else {
            state.clone()
        };
        let (started_at, speed_bps) = self.compute_timing(job_id, done, now, &effective_state);
        let eta_seconds = if speed_bps > 0 && total > done && !matches!(effective_state, TransferState::Paused) {
            Some((total - done).div_ceil(speed_bps))
        } else {
            None
        };

        let status = TransferStatus {
            job_id: job_id.to_string(),
            file_name: file_name.to_string(),
            state: effective_state.clone(),
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
                s.paused.remove(job_id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TransferPhase;

    #[test]
    fn detects_active_transfers() {
        let hub = ProgressHub::new();
        assert!(!hub.has_active_transfers());

        hub.transition(
            "job-1",
            "file.bin",
            TransferState::Running,
            TransferPhase::Uploading,
            None,
            10,
            100,
            None,
        );
        assert!(hub.has_active_transfers());

        hub.transition(
            "job-1",
            "file.bin",
            TransferState::Completed,
            TransferPhase::Completed,
            None,
            100,
            100,
            None,
        );
        assert!(!hub.has_active_transfers());
    }
}
