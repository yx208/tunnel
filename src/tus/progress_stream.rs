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
        update_threshold_time: Duration,
        initial_offset: u64,
    }
}

impl<S> AggregatedProgressStream<S> {
    pub fn new(inner: S, upload_id: UploadId, aggregator: Arc<ProgressAggregator>, initial_offset: u64) -> Self {
        let bytes_uploaded = Arc::new(AtomicU64::new(initial_offset));
        
        // 立即发送初始进度
        if initial_offset > 0 {
            aggregator.update_task_progress(upload_id, initial_offset);
        }
        
        Self {
            inner,
            upload_id,
            aggregator,
            bytes_uploaded,
            last_update_bytes: initial_offset,
            last_update_time: Instant::now(),
            update_threshold_bytes: 256 * 1024,  // 256KB
            update_threshold_time: Duration::from_millis(100),  // 100ms
            initial_offset,
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

    fn send_update(&mut self, current_bytes: u64, force: bool) {
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
                    // 更新已上传字节数
                    let new_total = this.bytes_uploaded.fetch_add(bytes_len as u64, Ordering::Relaxed) + bytes_len as u64;
                    
                    // 检查是否需要发送更新
                    let bytes_diff = new_total.saturating_sub(*this.last_update_bytes);
                    let time_diff = this.last_update_time.elapsed();

                    if bytes_diff >= *this.update_threshold_bytes || time_diff >= *this.update_threshold_time {
                        this.aggregator.update_task_progress(*this.upload_id, new_total);
                        *this.last_update_bytes = new_total;
                        *this.last_update_time = Instant::now();
                    }
                }
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(None) => {
                // 流结束时，发送最终更新
                let final_bytes = this.bytes_uploaded.load(Ordering::Relaxed);
                this.aggregator.update_task_progress(*this.upload_id, final_bytes);
                Poll::Ready(None)
            }
            Poll::Ready(Some(Err(e))) => {
                // 错误时也发送当前进度
                let current_bytes = this.bytes_uploaded.load(Ordering::Relaxed);
                this.aggregator.update_task_progress(*this.upload_id, current_bytes);
                Poll::Ready(Some(Err(e)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
