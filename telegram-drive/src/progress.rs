use crate::models::{TransferState, TransferStatus};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct ProgressHub {
    tx: broadcast::Sender<TransferStatus>,
    cancelled: Arc<Mutex<HashSet<String>>>,
}

impl ProgressHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(4096);
        Self {
            tx,
            cancelled: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TransferStatus> {
        self.tx.subscribe()
    }

    pub fn publish(&self, status: TransferStatus) {
        let _ = self.tx.send(status);
    }

    pub fn cancel(&self, job_id: &str) {
        if let Ok(mut c) = self.cancelled.lock() {
            c.insert(job_id.to_string());
        }
    }

    pub fn is_cancelled(&self, job_id: &str) -> bool {
        self.cancelled
            .lock()
            .map(|c| c.contains(job_id))
            .unwrap_or(true)
    }

    pub fn transition(&self, job_id: &str, file_name: &str, state: TransferState, done: u64, total: u64, err: Option<String>) {
        self.publish(TransferStatus {
            job_id: job_id.to_string(),
            file_name: file_name.to_string(),
            state,
            bytes_done: done,
            bytes_total: total,
            error: err,
        });
    }
}
