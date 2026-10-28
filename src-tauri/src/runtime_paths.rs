use crate::AppPaths;
use std::{
    fmt,
    ops::Deref,
    sync::{Arc, Condvar, Mutex},
};

#[derive(Clone)]
pub struct RuntimePaths {
    inner: Arc<RuntimePathsInner>,
}

struct RuntimePathsInner {
    state: Mutex<RuntimePathsState>,
    idle: Condvar,
}

struct RuntimePathsState {
    paths: AppPaths,
    maintenance: bool,
    active_leases: usize,
}

impl RuntimePaths {
    pub fn new(paths: AppPaths) -> Self {
        Self {
            inner: Arc::new(RuntimePathsInner {
                state: Mutex::new(RuntimePathsState {
                    paths,
                    maintenance: false,
                    active_leases: 0,
                }),
                idle: Condvar::new(),
            }),
        }
    }

    pub fn acquire(&self) -> Result<PathLease, String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "runtime_paths_lock_poisoned".to_string())?;
        if state.maintenance {
            return Err("data_root_migration_in_progress".to_string());
        }
        state.active_leases = state
            .active_leases
            .checked_add(1)
            .ok_or_else(|| "runtime_path_lease_overflow".to_string())?;
        Ok(PathLease {
            runtime: self.clone(),
            paths: state.paths.clone(),
            released: false,
        })
    }

    pub fn begin_maintenance(&self) -> Result<MaintenanceLease, String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "runtime_paths_lock_poisoned".to_string())?;
        if state.maintenance {
            return Err("data_root_migration_in_progress".to_string());
        }
        state.maintenance = true;
        while state.active_leases > 0 {
            state = self
                .inner
                .idle
                .wait(state)
                .map_err(|_| "runtime_paths_lock_poisoned".to_string())?;
        }
        Ok(MaintenanceLease {
            runtime: self.clone(),
            current_paths: state.paths.clone(),
            finished: false,
        })
    }

    pub fn maintenance_active(&self) -> Result<bool, String> {
        self.inner
            .state
            .lock()
            .map(|state| state.maintenance)
            .map_err(|_| "runtime_paths_lock_poisoned".to_string())
    }

    fn release(&self) {
        let Ok(mut state) = self.inner.state.lock() else {
            return;
        };
        if state.active_leases == 0 {
            return;
        }
        state.active_leases -= 1;
        if state.active_leases == 0 {
            self.inner.idle.notify_all();
        }
    }

    fn finish_maintenance(&self, paths: Option<AppPaths>) -> Result<(), String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "runtime_paths_lock_poisoned".to_string())?;
        if !state.maintenance || state.active_leases != 0 {
            return Err("runtime_paths_maintenance_mismatch".to_string());
        }
        if let Some(paths) = paths {
            state.paths = paths;
        }
        state.maintenance = false;
        self.inner.idle.notify_all();
        Ok(())
    }
}

pub struct PathLease {
    runtime: RuntimePaths,
    paths: AppPaths,
    released: bool,
}

pub type BackgroundPathLease = PathLease;

impl Deref for PathLease {
    type Target = AppPaths;

    fn deref(&self) -> &Self::Target {
        &self.paths
    }
}

impl fmt::Debug for PathLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PathLease")
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

impl Drop for PathLease {
    fn drop(&mut self) {
        if !self.released {
            self.runtime.release();
            self.released = true;
        }
    }
}

pub struct MaintenanceLease {
    runtime: RuntimePaths,
    current_paths: AppPaths,
    finished: bool,
}

impl MaintenanceLease {
    pub fn current_paths(&self) -> &AppPaths {
        &self.current_paths
    }

    pub fn commit(mut self, paths: AppPaths) -> Result<(), String> {
        self.runtime.finish_maintenance(Some(paths))?;
        self.finished = true;
        Ok(())
    }
}

impl fmt::Debug for MaintenanceLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MaintenanceLease")
            .field("current_paths", &self.current_paths)
            .finish_non_exhaustive()
    }
}

impl Drop for MaintenanceLease {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.runtime.finish_maintenance(None);
            self.finished = true;
        }
    }
}
