use bytes::Bytes;
use futures::Stream;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use super::progress_aggregator::ProgressAggregator;
use super::types::UploadId;

pin_project! {
    /// 进度流包装器 - 与进度聚合器集成
    pub struct AggregatedProgressStream<S> {
        #[pin]
        inner: S,
        upload_id: UploadId,
        aggregator: Arc<ProgressAggregator>,
        bytes_uploaded: u64,
    }
}

impl<S> AggregatedProgressStream<S> {
    pub fn new(
        inner: S,
        upload_id: UploadId,
        aggregator: Arc<ProgressAggregator>,
        initial_offset: u64,
    ) -> Self {
        Self {
            inner,
            upload_id,
            aggregator,
            bytes_uploaded: initial_offset,
        }
    }
}

impl<S> Stream for AggregatedProgressStream<S>
where
    S: Stream<Item = std::io::Result<Bytes>>,
{
    type Item = std::io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        match this.inner.poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                let bytes_len = chunk.len() as u64;
                if bytes_len > 0 {
                    *this.bytes_uploaded += bytes_len;
                    // 更新到聚合器
                    this.aggregator
                        .update_task_progress(*this.upload_id, *this.bytes_uploaded);
                }
                Poll::Ready(Some(Ok(chunk)))
            }
            other => other,
        }
    }
}
