use tokio::time::{Instant as TokioInstant, sleep_until};

pub(super) fn batch_dispatch_spacing(batch_start_delay_ms: u64) -> std::time::Duration {
    std::time::Duration::from_millis(batch_start_delay_ms)
}

pub(super) async fn wait_for_dispatch_slot(
    next_dispatch_at: &mut Option<TokioInstant>,
    dispatch_spacing: std::time::Duration,
) {
    if let Some(deadline) = *next_dispatch_at {
        sleep_until(deadline).await;
    }
    *next_dispatch_at = Some(TokioInstant::now() + dispatch_spacing);
}
