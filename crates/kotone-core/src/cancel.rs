//! Cooperative cancellation shared by text processing and assistant requests.

use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct CancellationToken {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    cancelled: Mutex<bool>,
    notify: Notify,
}

impl CancellationToken {
    pub fn cancel(&self) {
        let mut cancelled = self.inner.cancelled.lock().unwrap();
        if !*cancelled {
            *cancelled = true;
            self.inner.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        *self.inner.cancelled.lock().unwrap()
    }

    pub async fn cancelled(&self) {
        // Register before checking the flag: notify_waiters does not retain a
        // permit for a future waiter, so check-then-await can miss cancellation.
        let notified = self.inner.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if !self.is_cancelled() {
            notified.await;
        }
    }

    /// Linearize a short synchronous commit with cancellation. Do not block on
    /// an async task or call this token again from inside the closure.
    pub fn run_if_active<T>(&self, commit: impl FnOnce() -> T) -> Option<T> {
        let cancelled = self.inner.cancelled.lock().unwrap();
        if *cancelled {
            None
        } else {
            Some(commit())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancellation_wakes_existing_and_future_waiters() {
        let token = CancellationToken::default();
        let other = token.clone();
        let waiter = tokio::spawn(async move { other.cancelled().await });
        tokio::task::yield_now().await;
        token.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), token.cancelled())
            .await
            .unwrap();
        assert!(token.run_if_active(|| panic!("cancelled commit")).is_none());
    }
}
