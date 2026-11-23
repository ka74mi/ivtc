use std::sync::atomic::{AtomicUsize, Ordering};

static LIVE_OBJECTS: AtomicUsize = AtomicUsize::new(0);
static SERVER_LOCKS: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct ModuleRef;

impl ModuleRef {
    pub(crate) fn new() -> Self {
        LIVE_OBJECTS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for ModuleRef {
    fn drop(&mut self) {
        LIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(crate) fn lock_server(lock: bool) {
    if lock {
        SERVER_LOCKS.fetch_add(1, Ordering::SeqCst);
        return;
    }

    let mut current = SERVER_LOCKS.load(Ordering::SeqCst);
    while current > 0 {
        match SERVER_LOCKS.compare_exchange_weak(
            current,
            current - 1,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
}

pub(crate) fn can_unload() -> bool {
    LIVE_OBJECTS.load(Ordering::SeqCst) == 0 && SERVER_LOCKS.load(Ordering::SeqCst) == 0
}
