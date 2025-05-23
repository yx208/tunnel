use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::Mutex;

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

    /// 瞬时速度计算
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
    callback: Option<ProgressCallback>,
    update_interval: Duration,
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
}
