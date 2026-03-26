//! Shared helpers for pacing concurrent LLM requests.

use std::{future::Future, sync::Arc, time::Duration};

use tokio::{
    task::JoinSet,
    time::{Instant as TokioInstant, sleep_until},
};

use crate::error::{AppError, Result};

/// Default delay between dispatching concurrent LLM requests.
pub const DEFAULT_REQUEST_DISPATCH_DELAY_MS: u64 = 500;

/// Controls how delayed concurrent requests are dispatched.
#[derive(Debug, Clone, Copy)]
pub struct RequestBatchOptions {
    pub max_concurrency: usize,
    pub dispatch_delay_ms: u64,
}

impl RequestBatchOptions {
    #[must_use]
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            max_concurrency: max_concurrency.max(1),
            dispatch_delay_ms: DEFAULT_REQUEST_DISPATCH_DELAY_MS,
        }
    }

    #[must_use]
    pub fn with_dispatch_delay(mut self, dispatch_delay_ms: u64) -> Self {
        self.dispatch_delay_ms = dispatch_delay_ms;
        self
    }

    #[must_use]
    fn normalized(self) -> Self {
        Self {
            max_concurrency: self.max_concurrency.max(1),
            dispatch_delay_ms: self.dispatch_delay_ms,
        }
    }
}

impl Default for RequestBatchOptions {
    fn default() -> Self {
        Self::new(1)
    }
}

pub fn batch_dispatch_spacing(batch_start_delay_ms: u64) -> Duration {
    Duration::from_millis(batch_start_delay_ms)
}

pub async fn wait_for_dispatch_slot(
    next_dispatch_at: &mut Option<TokioInstant>,
    dispatch_spacing: Duration,
) {
    if let Some(deadline) = *next_dispatch_at {
        sleep_until(deadline).await;
    }
    *next_dispatch_at = Some(TokioInstant::now() + dispatch_spacing);
}

/// Executes async request jobs concurrently while spacing out dispatches.
///
/// Results are returned in the same order as the input sequence, even though
/// requests may complete out of order.
pub async fn run_delayed_concurrent_requests<I, T, F, Fut>(
    requests: impl IntoIterator<Item = I>,
    options: RequestBatchOptions,
    run_request: F,
) -> Result<Vec<T>>
where
    I: Send + 'static,
    T: Send + 'static,
    F: Fn(I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<T>> + Send + 'static,
{
    let options = options.normalized();
    let requests: Vec<I> = requests.into_iter().collect();
    if requests.is_empty() {
        return Ok(Vec::new());
    }

    let request_count = requests.len();
    let dispatch_spacing = batch_dispatch_spacing(options.dispatch_delay_ms);
    let run_request = Arc::new(run_request);
    let mut pending_requests = requests.into_iter().enumerate();
    let mut in_flight = JoinSet::new();
    let mut next_dispatch_at = None;
    let mut completed = Vec::with_capacity(request_count);

    loop {
        while in_flight.len() < options.max_concurrency {
            let Some((index, request)) = pending_requests.next() else {
                break;
            };
            wait_for_dispatch_slot(&mut next_dispatch_at, dispatch_spacing).await;

            let run_request = Arc::clone(&run_request);
            in_flight
                .spawn(async move { run_request(request).await.map(|response| (index, response)) });
        }

        let Some(joined) = in_flight.join_next().await else {
            break;
        };

        match joined {
            Ok(Ok(result)) => completed.push(result),
            Ok(Err(err)) => {
                in_flight.abort_all();
                return Err(err);
            }
            Err(err) => {
                in_flight.abort_all();
                return Err(AppError::Execution(format!(
                    "delayed concurrent request task failed: {err}"
                )));
            }
        }
    }

    completed.sort_by_key(|(index, _)| *index);
    Ok(completed
        .into_iter()
        .map(|(_, response)| response)
        .collect())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use tokio::time::{Instant, sleep};

    use super::*;

    #[test]
    fn request_batch_options_default_to_500ms_delay() {
        let options = RequestBatchOptions::default();

        assert_eq!(options.max_concurrency, 1);
        assert_eq!(options.dispatch_delay_ms, DEFAULT_REQUEST_DISPATCH_DELAY_MS);
    }

    #[tokio::test]
    async fn delayed_concurrent_requests_preserve_order_and_dispatch_spacing() {
        let start_times = Arc::new(Mutex::new(Vec::new()));
        let current_in_flight = Arc::new(AtomicUsize::new(0));
        let max_in_flight = Arc::new(AtomicUsize::new(0));

        let results = run_delayed_concurrent_requests(
            vec![0_usize, 1, 2],
            RequestBatchOptions::new(3).with_dispatch_delay(20),
            {
                let start_times = Arc::clone(&start_times);
                let current_in_flight = Arc::clone(&current_in_flight);
                let max_in_flight = Arc::clone(&max_in_flight);
                move |index| {
                    let start_times = Arc::clone(&start_times);
                    let current_in_flight = Arc::clone(&current_in_flight);
                    let max_in_flight = Arc::clone(&max_in_flight);
                    async move {
                        start_times
                            .lock()
                            .expect("start time mutex should not be poisoned")
                            .push((index, Instant::now()));
                        let in_flight = current_in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                        update_max(&max_in_flight, in_flight);
                        sleep(Duration::from_millis(60)).await;
                        current_in_flight.fetch_sub(1, Ordering::SeqCst);
                        Ok(index)
                    }
                }
            },
        )
        .await
        .expect("delayed concurrent requests should succeed");

        assert_eq!(results, vec![0, 1, 2]);
        assert_eq!(max_in_flight.load(Ordering::SeqCst), 3);

        let mut starts = start_times
            .lock()
            .expect("start time mutex should not be poisoned")
            .clone();
        starts.sort_by_key(|(index, _)| *index);
        assert_eq!(starts.len(), 3);
        assert!(
            starts[1].1.duration_since(starts[0].1) >= Duration::from_millis(15),
            "second dispatch should be delayed"
        );
        assert!(
            starts[2].1.duration_since(starts[1].1) >= Duration::from_millis(15),
            "third dispatch should be delayed"
        );
    }

    #[tokio::test]
    async fn delayed_concurrent_requests_return_task_errors() {
        let err = run_delayed_concurrent_requests(
            vec![0_usize, 1, 2],
            RequestBatchOptions::new(2).with_dispatch_delay(1),
            |index| async move {
                if index == 1 {
                    return Err(AppError::Llm("rate limit".to_string()));
                }
                sleep(Duration::from_millis(10)).await;
                Ok(index)
            },
        )
        .await
        .expect_err("batching should surface request errors");

        match err {
            AppError::Llm(message) => assert_eq!(message, "rate limit"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    fn update_max(target: &AtomicUsize, candidate: usize) {
        let mut observed = target.load(Ordering::SeqCst);
        while candidate > observed {
            match target.compare_exchange(observed, candidate, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => break,
                Err(actual) => observed = actual,
            }
        }
    }
}
