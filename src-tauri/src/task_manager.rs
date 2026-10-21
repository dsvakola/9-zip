use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

#[derive(Clone, Default)]
pub struct TaskManager {
    tasks: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn register(&self, task_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut lock) = self.tasks.write() {
            lock.insert(task_id.to_string(), Arc::clone(&flag));
        }
        flag
    }

    pub fn unregister(&self, task_id: &str) {
        if let Ok(mut lock) = self.tasks.write() {
            lock.remove(task_id);
        }
    }

    pub fn cancel(&self, task_id: &str) -> bool {
        if let Some(flag) = self
            .tasks
            .read()
            .ok()
            .and_then(|lock| lock.get(task_id).cloned())
        {
            flag.store(true, Ordering::SeqCst);
            return true;
        }
        false
    }

    pub fn is_cancelled(flag: &Arc<AtomicBool>) -> bool {
        flag.load(Ordering::SeqCst)
    }
}
