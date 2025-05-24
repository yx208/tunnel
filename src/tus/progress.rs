use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use bytes::Bytes;
use futures::Stream;
use parking_lot::Mutex;
use pin_project_lite::pin_project;

pub struct ProgressInfo {
    /// 已上传字节数
    pub bytes_uploaded: u64,

    /// 总字节数
    pub total_bytes: u64,

    /// 瞬时速度 (bytes/sec) - 基于最近样本
    pub instant_speed: f64,

    /// 平均速度 (bytes/sec) - 基于总时间
    pub average_speed: f64,

    /// 预计剩余时间
    pub eta: Option<Duration>,

    /// 进度百分比
    pub percentage: f64,
}

pub type ProgressCallback = Arc<dyn Fn(ProgressInfo) + Sync + Send>;

/// 速度计算器 - 使用环形缓冲区
struct SpeedCalculator {
    /// 采样的时间跟字节
    samples: Vec<(Instant, u64)>,

    /// 最大采样
    max_samples: usize,

    /// 下一个要写入的位置
    current_index: usize,

    /// 累计所有传输的字节数，用于计算整体平均速度
    total_bytes: u64,

    /// 开始时间
    start_time: Instant,
}

impl SpeedCalculator {
    pub fn new(max_samples: usize) -> Self {
        Self {
            max_samples,
            samples: Vec::with_capacity(max_samples),
            current_index: 0,
            total_bytes: 0,
            start_time: Instant::now(),
        }
    }

    /// 收集和管理上传速度的样本数据
    fn add_sample(&mut self, bytes: u64) {
        let now = Instant::now();
        self.total_bytes += bytes;

        // 当不满采样最大量直接添加
        if self.samples.len() < self.max_samples {
            self.samples.push((now, bytes));
        } else {
            // 缓冲区已满时，使用环形缓冲区覆盖最老的样本
            self.samples[self.current_index] = (now, bytes);
            self.current_index = (self.current_index + 1) % self.max_samples;
        }
    }

    /// 瞬时速度
    fn instant_speed(&self) -> f64 {
        // 1 个或没有
        if self.samples.len() < 2 {
            return 0.0;
        }

        // 使用最近的 25% 样本计算瞬时速度，最少使用 2 个样本
        let recent_count = (self.samples.len() / 4).max(2);
        let recent_samples = if self.samples.len() == self.max_samples {
            // 环形缓冲区已满，需要正确获取最近的样本
            let start = (self.current_index + self.max_samples - recent_count) % self.max_samples;
            let mut samples = Vec::with_capacity(recent_count);
            for i in 0..recent_count {
                samples.push(self.samples[(start + i) % self.max_samples]);
            }

            samples
        } else {
            // 缓冲区未满，取 samples 后面的 recent_count 个样本
            self.samples[self.samples.len() - recent_count..].to_vec()
        };

        if recent_samples.len() < 2 {
            return 0.0;
        }

        let total_bytes: u64 = recent_samples.iter().map(|(_, b)| b).sum();
        // 计算第一个跟最后一个样本的时间差
        let duration = recent_samples.last().unwrap().0
            .duration_since(recent_samples.first().unwrap().0)
            .as_secs_f64();

        if duration > 0.0 {
            total_bytes as f64 / duration
        } else {
            0.0
        }
    }

    /// 平均速度
    fn average_speed(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.total_bytes as f64 / elapsed
        } else {
            0.0
        }
    }
}

/// 进度追踪器
pub struct ProgressTracker {
    total_bytes: u64,
    initial_offset: u64,
    bytes_transferred: Arc<Mutex<u64>>,
    speed_calc: Arc<Mutex<SpeedCalculator>>,
    last_update: Arc<Mutex<Instant>>,
    update_interval: Duration,
    pub callback: Option<ProgressCallback>,
}

impl ProgressTracker {
    pub fn new(total_bytes: u64, initial_offset: u64) -> Self {
        Self {
            total_bytes,
            initial_offset,
            bytes_transferred: Arc::new(Mutex::new(0)),
            speed_calc: Arc::new(Mutex::new(SpeedCalculator::new(20))),
            last_update: Arc::new(Mutex::new(Instant::now())),
            callback: None,
            update_interval: Duration::from_secs(1),
        }
    }

    pub fn with_callback(mut self, callback: ProgressCallback) -> Self
    {
        self.callback = Some(callback);
        self
    }

    pub fn with_update_interval(mut self, interval: Duration) -> Self {
        self.update_interval = interval;
        self
    }

    /// 累加字节
    fn record_bytes(&self, bytes: u64) {
        let mut bytes_transferred = self.bytes_transferred.lock();
        *bytes_transferred += bytes;

        let total_transferred = *bytes_transferred;
        drop(bytes_transferred);

        let now = Instant::now();

        // 快速检查是否需要更新
        {
            let last_update = self.last_update.lock();
            if now.duration_since(*last_update) < self.update_interval {
                return;
            }
        }

        let mut last_update = self.last_update.lock();
        let mut speed_calc = self.speed_calc.lock();

        // 双重检查（避免竞态条件）
        if now.duration_since(*last_update) >= self.update_interval {
            speed_calc.add_sample(bytes);

            let actual_uploaded = self.initial_offset + total_transferred;
            let percentage = (actual_uploaded as f64 / self.total_bytes as f64) * 100.0;
            let instant_speed = speed_calc.instant_speed();
            let average_speed = speed_calc.average_speed();

            let remaining_bytes = self.total_bytes.saturating_sub(actual_uploaded);

            // 预计剩余时间
            let eta = if instant_speed > 0.0 {
                Some(Duration::from_secs_f64(remaining_bytes as f64 / instant_speed))
            } else if average_speed > 0.0 {
                Some(Duration::from_secs_f64(remaining_bytes as f64 / average_speed))
            } else {
                None
            };

            let info = ProgressInfo {
                bytes_uploaded: actual_uploaded,
                total_bytes: self.total_bytes,
                instant_speed,
                average_speed,
                eta,
                percentage
            };

            if let Some(ref callback) = self.callback {
                callback(info);
            }

            *last_update = now;
        }
    }
}

pin_project! {
    pub struct ProgressStream<S> {
        #[pin]
        inner: S,
        tracker: Arc<ProgressTracker>,
    }
}

impl <S> ProgressStream<S> {
    pub fn new(inner: S, tracker: Arc<ProgressTracker>) -> Self {
        Self { inner, tracker }
    }
}

impl<S> Stream for ProgressStream<S>
where 
    S: Stream<Item = std::io::Result<Bytes>>,
{
    type Item = std::io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        
        match this.inner.poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                let bytes_len = chunk.len();
                if bytes_len > 0 {
                    this.tracker.record_bytes(bytes_len as u64);
                }
                
                Poll::Ready(Some(Ok(chunk)))
            }
            other => other
        }
    }
}
