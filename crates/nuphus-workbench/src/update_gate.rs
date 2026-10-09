//! Atomic admission shared by every transport and long-lived native worker.
use crate::{ApiError, Result};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    active: usize,
    installing: bool,
}

#[derive(Default)]
pub struct UpdateGate(Mutex<State>);

pub struct Activity(Arc<UpdateGate>);
pub struct Installation(Arc<UpdateGate>);

impl UpdateGate {
    pub fn admit(self: &Arc<Self>) -> Result<Activity> {
        let mut state = self.0.lock().map_err(|_| unavailable())?;
        if state.installing {
            return Err(ApiError::new(
                "upgrade_in_progress",
                "灵雀正在安装更新，请稍后重连。",
            ));
        }
        state.active += 1;
        Ok(Activity(self.clone()))
    }

    /// Check idle and close admission under the SAME lock. Never cancel work.
    pub fn freeze(self: &Arc<Self>) -> Result<Installation> {
        let mut state = self.0.lock().map_err(|_| unavailable())?;
        if state.installing || state.active != 0 {
            return Err(ApiError::new(
                "update_busy",
                "仍有任务、保存或模型准备未结束，请稍后安装更新。",
            ));
        }
        state.installing = true;
        Ok(Installation(self.clone()))
    }

    pub fn installing(&self) -> bool {
        self.0.lock().map_or(true, |s| s.installing)
    }
}

fn unavailable() -> ApiError {
    ApiError::new(
        "update_gate_unavailable",
        "无法安全检查任务状态，请重启后重试。",
    )
}

impl Drop for Activity {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0 .0.lock() {
            state.active -= 1;
        }
    }
}

impl Drop for Installation {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0 .0.lock() {
            state.installing = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_worker_and_failed_installation_keep_work_safe() {
        let gate = Arc::new(UpdateGate::default());
        let request = gate.admit().unwrap();
        let worker = gate.admit().unwrap();
        drop(request);
        assert!(
            gate.freeze().is_err(),
            "worker outlives its request, including pause"
        );
        drop(worker);
        let install = gate.freeze().unwrap();
        assert!(gate.admit().is_err());
        assert!(gate.freeze().is_err());
        drop(install);
        assert!(gate.admit().is_ok(), "failure restores admission");
    }

    #[test]
    fn freeze_and_admission_have_no_check_then_act_race() {
        for _ in 0..64 {
            let gate = Arc::new(UpdateGate::default());
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let task_gate = gate.clone();
            let task_barrier = barrier.clone();
            let worker = std::thread::spawn(move || {
                task_barrier.wait();
                task_gate.admit()
            });
            barrier.wait();
            let install = gate.freeze();
            let activity = worker.join().unwrap();
            assert!(!(install.is_ok() && activity.is_ok()));
        }
    }
}
