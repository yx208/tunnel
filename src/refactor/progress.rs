use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex, RwLock, broadcast};
use bytes::Bytes;
use futures::Stream;
use pin_project_lite::pin_project;
use tokio_util::sync::CancellationToken;
use crate::TransferId;

pin_project! {
    pub struct FileStream<S> {
        #[pin]
        inner: S,
        progress_tx: Option<mpsc::UnboundedSender<u64>>,
    }
}

impl<S> FileStream<S> {
    pub fn new(inner: S, progress_tx: Option<mpsc::UnboundedSender<u64>>) -> Self {
        Self { inner, progress_tx }
    }
}

impl<S> Stream for FileStream<S>
where
    S: Stream<Item = std::io::Result<Bytes>>,
{
    type Item = std::io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let project = self.project();
        match project.inner.poll_next(cx) {
            Poll::Ready(Some(Ok(item))) => {
                if let Some(tx) = project.progress_tx.as_ref() {
                    let bytes_length = item.len();
                    let _ = tx.send(bytes_length as u64);
                }
                
                Poll::Ready(Some(Ok(item)))
            },
            other => other
        }
    }
}

struct TaskProgress {
    bytes_transferred: u64,
    total_bytes: u64,
    start_time: Instant,
    last_update: Instant,
}

impl Default for TaskProgress {
    fn default() -> Self {
        Self {
            bytes_transferred: 0,
            total_bytes: 0,
            start_time: Instant::now(),
            last_update: Instant::now(),
        }
    }
}

impl TaskProgress {
    fn update(&mut self, bytes: u64) {
        self.last_update = Instant::now();
        self.bytes_transferred += bytes;
    }

    pub async fn get_stats(&self) -> TransferStats {
        TransferStats {
            start_time: self.start_time.clone(),
            end_time: None,
            bytes_transferred: self.bytes_transferred,
            total_bytes: self.total_bytes,
            current_speed: 0.0,
            average_speed: 0.0,
            eta: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TransferStats {
    start_time: Instant,
    end_time: Option<Instant>,
    bytes_transferred: u64,
    total_bytes: u64,
    current_speed: f64,
    average_speed: f64,
    eta: Option<Duration>,
}

struct SpeedTrackerHandle {
    tracker: Arc<Mutex<TaskProgress>>,
    handle: tokio::task::JoinHandle<()>,
}

impl SpeedTrackerHandle {
    pub fn new(mut progress_rx: mpsc::UnboundedReceiver<u64>) -> SpeedTrackerHandle {
        let tracker = Arc::new(Mutex::new(TaskProgress::default()));

        let tracker_clone = tracker.clone();
        let handle = tokio::spawn(async move {
            while let Some(bytes) = progress_rx.recv().await {
                let mut guard = tracker_clone.lock().await;
                guard.update(bytes);
            }
        });

        SpeedTrackerHandle {
            tracker,
            handle
        }
    }

    pub fn cancel(&self) {
        self.handle.abort();
    }
}

async fn report_speed(aggregator: ProgressAggregator) {
    loop {
        tokio::select! {
            _ = aggregator.cancellation_token.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                let guard = aggregator.progress_tracker.read().await;
                if guard.len() == 0 {
                    continue;
                }
                
                let mut stats_vec = Vec::new();
                for item in guard.iter() {
                    let stats = item.1.tracker.lock().await.get_stats().await;
                    stats_vec.push((item.0.clone(), stats));
                }
                
                let _ = aggregator.stats_notify.send(stats_vec);
            }
        }
    }
}

#[derive(Clone)]
pub struct ProgressAggregator {
    progress_tracker: Arc<RwLock<HashMap<TransferId, SpeedTrackerHandle>>>,
    cancellation_token: CancellationToken,
    stats_notify: broadcast::Sender<Vec<(TransferId, TransferStats)>>,
}

impl ProgressAggregator {
    pub fn new(token: CancellationToken, enable_report: bool) -> Self {
        let (stats_notify, _) = broadcast::channel(64);
        
        let aggregator = Self {
            progress_tracker: Arc::new(RwLock::new(HashMap::new())),
            cancellation_token: token,
            stats_notify
        };
        
        if enable_report {
            tokio::spawn(report_speed(aggregator.clone()));
        }

        aggregator
    }

    pub async fn registry_task(&self, transfer_id: TransferId) -> mpsc::UnboundedSender<u64> {
        let mut guard = self.progress_tracker.write().await;
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();
        guard.insert(transfer_id, SpeedTrackerHandle::new(progress_rx));

        progress_tx
    }

    pub async fn unregister_task(&self, transfer_id: TransferId) {
        let mut guard = self.progress_tracker.write().await;
        if let Some(handle) = guard.remove(&transfer_id) {
            handle.cancel();
        }
    }
    
    pub fn subscribe(&self) -> broadcast::Receiver<Vec<(TransferId, TransferStats)>> {
        self.stats_notify.subscribe()
    }
}







