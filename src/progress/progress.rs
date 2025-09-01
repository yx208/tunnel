use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex, RwLock, broadcast};
use bytes::Bytes;
use futures::Stream;
use pin_project_lite::pin_project;
use tokio_util::sync::CancellationToken;
use crate::{TransferEvent, TransferId, TransferStats};

pin_project! {
    pub struct FileStream<S> {
        #[pin]
        inner: S,
        bytes_tx: mpsc::UnboundedSender<u64>,
        read_bytes: Option<u64>,
    }
}

impl<S> FileStream<S> {
    pub fn new(inner: S, bytes_tx: mpsc::UnboundedSender<u64>) -> Self {
        Self {
            inner,
            bytes_tx,
            read_bytes: None,
        }
    }
}

impl<S> Stream for FileStream<S>
where
    S: Stream<Item = std::io::Result<Bytes>>,
{
    type Item = std::io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut project = self.project();
        match project.inner.poll_next(cx) {
            Poll::Ready(Some(Ok(item))) => {
                if project.read_bytes.is_some() {
                    let _ = project.bytes_tx.send(project.read_bytes.unwrap());
                }

                *project.read_bytes = Some(item.len() as u64);

                Poll::Ready(Some(Ok(item)))
            },
            Poll::Ready(None) => {
                let _ = project.bytes_tx.send(project.read_bytes.unwrap());
                *project.read_bytes = Some(0);
                Poll::Ready(None)
            },
            other => other
        }
    }
}

struct TaskProgress {
    samples: VecDeque<(u64, Instant)>,
    max_samples: usize,
    window_time: Duration,
    bytes_transferred: u64,
    total_bytes: u64,
    bytes_last_update: Instant,
    start_time: Instant,
    last_update: Instant,
}

impl TaskProgress {
    pub fn new(total_bytes: u64) -> Self {
        let now = Instant::now();
        Self {
            bytes_transferred: 0,
            total_bytes,
            window_time: Duration::from_secs(5),
            samples: VecDeque::with_capacity(5),
            max_samples: 5,
            bytes_last_update: now.clone(),
            start_time: now.clone(),
            last_update: now,
        }
    }

    fn update(&mut self, bytes: u64) {
        let now = Instant::now();
        self.bytes_transferred += bytes;

        // 小于一秒不更新
        if now.duration_since(self.last_update).as_secs_f64() < 1.0 {
            self.bytes_last_update = now;
            return;
        }

        self.samples.push_back((self.bytes_transferred, now));
        if self.samples.len() > self.max_samples {
            self.samples.pop_front();
        }

        let cutoff = now - self.window_time;
        self.samples.retain(|(_, t)| *t > cutoff);
        self.last_update = now;
        self.bytes_last_update = now;
    }

    pub async fn get_stats(&self) -> TransferStats {
        let instant_speed = match self.samples.len() {
            len if len >= 2 => {
                let (first_bytes, first_time) = self.samples.front().unwrap();
                let (last_bytes, last_time) = self.samples.back().unwrap();

                let bytes_diff = last_bytes.saturating_sub(*first_bytes);
                let time_diff = last_time.duration_since(*first_time).as_secs_f64();

                if time_diff > 0.0 {
                    bytes_diff as f64 / time_diff
                } else {
                    0.0
                }
            }
            _ => {
                0.0
            }
        };

        let average_speed = self.bytes_transferred as f64 / self.start_time.elapsed().as_secs_f64();

        TransferStats {
            start_time: self.start_time.clone(),
            end_time: None,
            bytes_transferred: self.bytes_transferred,
            total_bytes: self.total_bytes,
            instant_speed,
            average_speed,
            eta: None,
        }
    }
}

pub struct SpeedTracker {
    tracker: Arc<Mutex<TaskProgress>>,
    handle: tokio::task::JoinHandle<()>,
}

impl SpeedTracker {
    pub fn new(mut bytes_rx: mpsc::UnboundedReceiver<u64>) -> SpeedTracker {
        let tracker = Arc::new(Mutex::new(TaskProgress::new(0)));

        let tracker_clone = tracker.clone();
        let handle = tokio::spawn(async move {
            while let Some(bytes) = bytes_rx.recv().await {
                let mut guard = tracker_clone.lock().await;
                guard.update(bytes);
            }
        });

        SpeedTracker {
            tracker,
            handle
        }
    }

    pub fn cancel(&self) {
        self.handle.abort();
    }
}

pub struct ProgressAggregator {
    pub trackers: Arc<RwLock<HashMap<TransferId, SpeedTracker>>>,
    pub event_tx: broadcast::Sender<TransferEvent>,
    pub cancellation_token: CancellationToken,
}

impl ProgressAggregator {
    pub fn enable_report(mut self) -> Self {
        let trackers = self.trackers.clone();
        let event_tx = self.event_tx.clone();
        
        let cancellation_token = self.cancellation_token.clone();
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancellation_token.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                        let mut stats_vec = Vec::new();
                        let trackers_guard = trackers.read().await;
                        for item in trackers_guard.iter() {
                            let stats = item.1.tracker.lock().await.get_stats().await;
                            stats_vec.push((item.0.clone(), stats));
                        }
                        
                        let _ = event_tx.send(TransferEvent::Progress {
                            updates: stats_vec
                        });
                    }
                }
            }
        });

        self
    }

    pub async fn registry_task(&self, id: TransferId, bytes_rx: mpsc::UnboundedReceiver<u64>) {
        let tracker = SpeedTracker::new(bytes_rx);
        let mut trackers_guard = self.trackers.write().await;
        trackers_guard.insert(id, tracker);
    }

    pub async fn unregister_task(&self, transfer_id: &TransferId) {
        let mut trackers_guard = self.trackers.write().await;
        if let Some(handle) = trackers_guard.remove(transfer_id) {
            handle.cancel();
        }
    }

    pub async fn clear(&mut self) {
        let mut trackers_guard = self.trackers.write().await;
        trackers_guard.clear();
    }
}
