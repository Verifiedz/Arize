//! The module write lock (ADR 0014): serialises a module's read-check-write sequences (ADR 0008,
//! "Concurrency"). One type for every module, so a module never keeps its own copy.

use std::sync::{Mutex, MutexGuard};

/// Held around each read-check-write sequence in a module, so two requests can't both read the
/// old state and then both write. It guards no data, so a lock poisoned by a panicking holder
/// carries no information: [`WriteLock::lock`] recovers it rather than failing every later
/// request.
///
/// Only for that pattern. A lock that guards real data, where poisoning does mean something,
/// should stay a plain `Mutex`.
#[derive(Debug, Default)]
pub struct WriteLock(Mutex<()>);

impl WriteLock {
    pub fn lock(&self) -> MutexGuard<'_, ()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn a_holder_that_panicked_does_not_lock_everyone_else_out() {
        let lock = Arc::new(WriteLock::default());
        let held = Arc::clone(&lock);
        let panicked = std::thread::spawn(move || {
            let _guard = held.lock();
            panic!("a request failed while holding the lock");
        })
        .join();
        assert!(panicked.is_err());
        assert!(lock.0.is_poisoned(), "the panic did poison the mutex");
        drop(lock.lock()); // and the next request still gets in
    }

    #[test]
    fn it_serialises_holders() {
        let lock = Arc::new(WriteLock::default());
        let counter = Arc::new(Mutex::new(Vec::new()));
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let (lock, counter) = (Arc::clone(&lock), Arc::clone(&counter));
                std::thread::spawn(move || {
                    let _guard = lock.lock();
                    // Read-check-write: no other holder can interleave between these.
                    let len = counter.lock().unwrap().len();
                    std::thread::yield_now();
                    counter.lock().unwrap().push((i, len));
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let mut seen: Vec<usize> = counter.lock().unwrap().iter().map(|(_, len)| *len).collect();
        seen.sort();
        assert_eq!(seen, (0..8).collect::<Vec<_>>(), "each holder saw every earlier write");
    }
}
