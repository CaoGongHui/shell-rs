//! Work on the core's runtime, tied to the call that started it.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use crate::failure::SshError;

/// Run `future` on `runtime` and wait for it. Swift cancelling its task or
/// Kotlin its coroutine drops the call's future, which stops the work too
/// rather than leaving it running unseen. A panic comes back as an error.
pub(crate) async fn run<T: Send + 'static>(
    runtime: &Handle,
    future: impl Future<Output = Result<T, SshError>> + Send + 'static,
) -> Result<T, SshError> {
    match AbortOnDrop(runtime.spawn(future)).await {
        Ok(result) => result,
        // The core is gone, and its runtime with it.
        Err(error) if error.is_cancelled() => Err(SshError::Closed),
        Err(panic) => Err(SshError::Internal {
            detail: panic.to_string(),
        }),
    }
}

/// A task that ends when its handle is dropped.
pub(crate) struct AbortOnDrop<T>(pub(crate) JoinHandle<T>);

impl<T> Future for AbortOnDrop<T> {
    type Output = Result<T, tokio::task::JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx)
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use crate::failure::SshError;

    /// Dropped when the task's future is.
    struct Dropped(Arc<AtomicBool>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// The app giving up on a call stops the work it started.
    #[test]
    fn a_call_given_up_on_stops_its_work() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        let work = {
            let guard = Dropped(dropped.clone());
            async move {
                let _guard = guard;
                std::future::pending::<()>().await;
                Ok::<_, SshError>(())
            }
        };
        // Started, then dropped unfinished, as a cancelled Swift task does.
        let call = super::run(runtime.handle(), work);
        assert!(futures::FutureExt::now_or_never(call).is_none());
        for _ in 0..100 {
            if dropped.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the work kept running");
    }
}
