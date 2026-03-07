use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;
use tracing::{info, warn};

#[derive(Clone)]
pub struct AppPerformanceController {
    tx: Sender<bool>,
}

impl AppPerformanceController {
    pub fn new() -> Self {
        #[cfg(target_os = "windows")]
        optimize_process_startup();

        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("tgdrive-performance".to_string())
            .spawn(move || performance_worker(rx))
            .expect("failed to spawn performance worker");

        Self { tx }
    }

    pub fn set_transfer_mode(&self, active: bool) {
        let _ = self.tx.send(active);
    }
}

fn performance_worker(rx: Receiver<bool>) {
    let mut current = false;
    loop {
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(next) => {
                if next != current {
                    apply_transfer_mode(next, true);
                    current = next;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if current {
                    apply_transfer_mode(true, false);
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if current {
                    apply_transfer_mode(false, true);
                }
                break;
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn optimize_process_startup() {
    use std::mem::size_of;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, ProcessPowerThrottling, SetPriorityClass, SetProcessInformation,
        ABOVE_NORMAL_PRIORITY_CLASS, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    };

    unsafe {
        let process = GetCurrentProcess();
        if SetPriorityClass(process, ABOVE_NORMAL_PRIORITY_CLASS) == 0 {
            warn!("failed to raise process priority class");
        } else {
            info!("process priority raised to ABOVE_NORMAL");
        }

        let throttling = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: 0,
        };
        if SetProcessInformation(
            process,
            ProcessPowerThrottling,
            &throttling as *const _ as *const _,
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        ) == 0
        {
            warn!("failed to disable process power throttling");
        } else {
            info!("process power throttling disabled");
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn optimize_process_startup() {
    info!("performance startup optimization not required on this platform");
}

#[cfg(target_os = "windows")]
fn apply_transfer_mode(active: bool, log_transition: bool) {
    use windows_sys::Win32::System::Power::{
        SetThreadExecutionState, ES_AWAYMODE_REQUIRED, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
    };

    let flags = if active {
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_AWAYMODE_REQUIRED
    } else {
        ES_CONTINUOUS
    };

    unsafe {
        if SetThreadExecutionState(flags) == 0 {
            warn!(active, "failed to update thread execution state");
        } else if log_transition && active {
            info!("transfer performance mode enabled");
        } else if log_transition {
            info!("transfer performance mode disabled");
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn apply_transfer_mode(active: bool, log_transition: bool) {
    if log_transition {
        info!(active, "transfer performance mode noop on this platform");
    }
}
