use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use bytes::Bytes;
use futures::Stream;
use pin_project_lite::pin_project;
use super::progress_aggregator::ProgressAggregator;
use super::types::UploadId;

pin_project! {
    pub struct AggregatedProgressStream<S> {
        #[pin]
        inner: S,
        upload_id: UploadId,
        aggregator: Arc<ProgressAggregator>,
        bytes_uploaded: Arc<AtomicU64>,

        last_update_bytes: u64,
        last_update_time: Instant,
        update_threshold_bytes: u64,
        update_threshold_time: Duration
    }
}

impl<S> AggregatedProgressStream<S> {
    pub fn new(inner: S, upload_id: UploadId, aggregator: Arc<ProgressAggregator>, initial_offset: u64) -> Self {
        let bytes_uploaded = Arc::new(AtomicU64::new(initial_offset));
        Self {
            inner,
            upload_id,
            aggregator,
            bytes_uploaded,
            last_update_bytes: initial_offset,
            last_update_time: Instant::now(),
            update_threshold_bytes: 256 * 1024,
            update_threshold_time: Duration::from_millis(100),
        }
    }

    fn should_update(&self, current_bytes: u64) -> bool {
        let bytes_diff = current_bytes.saturating_sub(self.last_update_bytes);
        if bytes_diff >= self.update_threshold_bytes {
            return true;
        }

        let time_diff = self.last_update_time.elapsed();
        time_diff >= self.update_threshold_time
    }

    fn send_update(&mut self, force: bool) {
        let current_bytes = self.bytes_uploaded.load(Ordering::Relaxed);

        if force || self.should_update(current_bytes) {
            self.aggregator.update_task_progress(self.upload_id, current_bytes);
            self.last_update_bytes = current_bytes;
            self.last_update_time = Instant::now();
        }
    }
}

impl<S> Stream for AggregatedProgressStream<S>
where
    S: Stream<Item = std::io::Result<Bytes>>
{
    type Item = std::io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        match this.inner.poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                let bytes_len = chunk.len();
                if bytes_len > 0 {
                    this.bytes_uploaded.fetch_add(bytes_len as u64, Ordering::Relaxed);

                    let current_bytes = this.bytes_uploaded.load(Ordering::Relaxed);

                    let bytes_diff = current_bytes.saturating_sub(*this.last_update_bytes);
                    let time_diff = this.last_update_time.elapsed();

                    if bytes_diff >= *this.update_threshold_bytes || time_diff >= *this.update_threshold_time {
                        this.aggregator.update_task_progress(*this.upload_id, current_bytes);
                        *this.last_update_bytes = current_bytes;
                        *this.last_update_time = Instant::now();
                    }
                }
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(None) => {
                let current_bytes = this.bytes_uploaded.load(Ordering::Relaxed);
                this.aggregator.update_task_progress(*this.upload_id, current_bytes);
                *this.last_update_bytes = current_bytes;
                *this.last_update_time = Instant::now();
                Poll::Ready(None)
            }
            other => other,
        }
    }
}
