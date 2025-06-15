use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use parking_lot::{Mutex, RwLock};
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use crate::tus::types::{AggregatedStats, BatchProgress, TaskProgress, UploadId};

/// 进度聚合器
pub struct ProgressAggregator {
    /// 任务注册表（只在注册/注销时使用）
    registry: Arc<RwLock<HashMap<UploadId, Arc<AtomicTaskTracker>>>>,

    /// 进度更新通道
    update_tx: crossbeam_channel::Sender<ProgressUpdate>,
    update_rx: crossbeam_channel::Receiver<ProgressUpdate>,

    /// 批量进度更新发送器
    batch_tx: mpsc::UnboundedSender<BatchProgress>,

    /// 更新间隔
    update_interval: Duration,

    /// 是否启用
    enabled: Arc<AtomicBool>,

    /// 活跃任务计数
    active_task_count: Arc<AtomicUsize>,

    /// 取消令牌
    cancellation_token: CancellationToken,
}

/// 进度更新消息
#[derive(Debug)]
struct ProgressUpdate {
    upload_id: UploadId,
    bytes_uploaded: u64,
    timestamp: Instant,
}

impl ProgressAggregator {
    pub fn new(batch_tx: mpsc::UnboundedSender<BatchProgress>) -> Self {
        let (update_tx, update_rx) = crossbeam_channel::unbounded();

        Self {
            registry: Arc::new(RwLock::new(HashMap::new())),
            update_rx,
            update_tx,
            batch_tx,
            update_interval: Duration::from_secs(1),
            enabled: Arc::new(AtomicBool::new(true)),
            active_task_count: Arc::new(AtomicUsize::new(0)),
            cancellation_token: CancellationToken::new(),
        }
    }

    /// 设置更新间隔
    pub fn with_update_interval(mut self, update_interval: Duration) -> Self {
        self.update_interval = update_interval;
        self
    }

    /// 注册新任务
    pub fn register_task(&self, upload_id: UploadId, total_bytes: u64) {
        let mut registry = self.registry.write();
        let tracker = Arc::new(AtomicTaskTracker::new(upload_id, total_bytes));
        registry.insert(upload_id, tracker);
        self.active_task_count.fetch_add(1, Ordering::SeqCst);
    }

    /// 注销任务
    pub fn unregister_task(&self, upload_id: UploadId) {
        let mut registry = self.registry.write();
        if registry.remove(&upload_id).is_some() {
            self.active_task_count.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// 启动
    pub fn start(self: Arc<Self>) -> ProgressAggregatorHandle {
        let registry = self.registry.clone();
        let update_rx = self.update_rx.clone();
        let batch_tx = self.batch_tx.clone();
        let enabled = self.enabled.clone();
        let update_interval = self.update_interval;
        let active_task_count = self.active_task_count.clone();
        let cancellation_token = self.cancellation_token.clone();

        // 启动更新收集任务（改为异步任务）
        let collector_handle = tokio::spawn({
            let registry = registry.clone();
            let cancellation_token = cancellation_token.clone();
            
            async move {
                let mut pending_updates: HashMap<UploadId, (u64, Instant)> = HashMap::new();
                
                loop {
                    // 使用 select! 来同时监听取消信号和更新
                    tokio::select! {
                        _ = cancellation_token.cancelled() => {
                            break;
                        }
                        _ = tokio::time::sleep(Duration::from_millis(10)) => {
                            // 批量收集更新
                            while let Ok(update) = update_rx.try_recv() {
                                pending_updates.insert(
                                    update.upload_id, 
                                    (update.bytes_uploaded, update.timestamp)
                                );
                                
                                // 如果收集了足够多的更新，立即处理
                                if pending_updates.len() >= 100 {
                                    break;
                                }
                            }

                            if !pending_updates.is_empty() {
                                let registry_guard = registry.read();
                                for (upload_id, (bytes, timestamp)) in pending_updates.drain() {
                                    if let Some(tracker) = registry_guard.get(&upload_id) {
                                        tracker.update(bytes, timestamp);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        // 启动统计报告任务
        let reporter_handle = tokio::spawn({
            let registry = registry.clone();
            let enabled = enabled.clone();
            let cancellation_token = cancellation_token.clone();

            async move {
                let mut interval = interval(update_interval);
                interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

                loop {
                    tokio::select! {
                        _ = cancellation_token.cancelled() => {
                            break;
                        }
                        _ = interval.tick() => {
                            if !enabled.load(Ordering::Relaxed) {
                                continue;
                            }

                            // 只在有活跃任务时才发送进度更新
                            if active_task_count.load(Ordering::Relaxed) == 0 {
                                continue;
                            }

                            let registry_read = registry.read();
                            if registry_read.is_empty() {
                                continue;
                            }

                            let mut task_progresses = Vec::new();
                            let mut total_bytes = 0u64;
                            let mut total_uploaded = 0u64;
                            let mut total_speed = 0.0;
                            let mut active_count = 0;

                            for tracker in registry_read.values() {
                                let stats = tracker.get_stats();

                                total_bytes += stats.total_bytes;
                                total_uploaded += stats.bytes_uploaded;
                                total_speed += stats.instant_speed;

                                if stats.percentage < 100.0 {
                                    active_count += 1;
                                }

                                task_progresses.push(TaskProgress {
                                    upload_id: stats.upload_id,
                                    bytes_uploaded: stats.bytes_uploaded,
                                    total_bytes: stats.total_bytes,
                                    instant_speed: stats.instant_speed,
                                    average_speed: stats.average_speed,
                                    percentage: stats.percentage,
                                    eta: stats.eta,
                                });
                            }

                            drop(registry_read);

                            // 计算总体统计
                            let overall_percentage = if total_bytes > 0 {
                                (total_uploaded as f64 / total_bytes as f64) * 100.0
                            } else {
                                0.0
                            };

                            let overall_eta = if total_speed > 0.0 {
                                let remaining = total_bytes.saturating_sub(total_uploaded);
                                Some(Duration::from_secs_f64(remaining as f64 / total_speed))
                            } else {
                                None
                            };

                            let aggregated_stats = AggregatedStats {
                                total_tasks: task_progresses.len(),
                                active_tasks: active_count,
                                total_bytes,
                                total_uploaded,
                                overall_speed: total_speed,
                                overall_percentage,
                                overall_eta,
                            };

                            let batch_progress = BatchProgress {
                                aggregated: aggregated_stats,
                                timestamp: Instant::now(),
                                tasks: task_progresses,
                            };

                            // 发送
                            let _ = batch_tx.send(batch_progress);
                        }
                    }
                }
            }
        });

        ProgressAggregatorHandle {
            aggregator: self,
            collector_handle,
            reporter_handle,
        }
    }

    /// 暂停
    pub fn pause(&self) {
        self.enabled.store(false, Ordering::Relaxed);
    }

    /// 恢复
    pub fn resume(&self) {
        self.enabled.store(true, Ordering::Relaxed);
    }

    /// 更新任务进度
    pub fn update_task_progress(&self, upload_id: UploadId, bytes_uploaded: u64) {
        let _ = self.update_tx.send(ProgressUpdate {
            upload_id,
            bytes_uploaded,
            timestamp: Instant::now(),
        });
    }

    /// 获取活跃任务数
    pub fn active_task_count(&self) -> usize {
        self.active_task_count.load(Ordering::Relaxed)
    }
}

struct AtomicTaskTracker {
    upload_id: UploadId,
    total_bytes: u64,
    bytes_uploaded: AtomicU64,
    start_time: Instant,
    speed_calculator: Mutex<SpeedCalculator>
}

impl AtomicTaskTracker {
    fn new(upload_id: UploadId, total_bytes: u64) -> Self {
        Self {
            upload_id,
            total_bytes,
            bytes_uploaded: AtomicU64::new(0),
            start_time: Instant::now(),
            speed_calculator: Mutex::new(SpeedCalculator::new())
        }
    }

    fn update(&self, bytes_uploaded: u64, timestamp: Instant) {
        // 更新总字节数
        self.bytes_uploaded.store(bytes_uploaded, Ordering::Relaxed);

        // 更新速度计算器
        let mut calculator = self.speed_calculator.lock();
        calculator.add_sample(bytes_uploaded, timestamp);
    }

    fn get_stats(&self) -> TaskStats {
        let bytes_uploaded = self.bytes_uploaded.load(Ordering::Relaxed);
        let mut calculator = self.speed_calculator.lock();

        let instant_speed = calculator.get_instant_speed();
        let average_speed = self.calculate_average_speed(bytes_uploaded);
        let percentage = self.calculate_percentage(bytes_uploaded);
        let eta = self.calculate_eta(bytes_uploaded, instant_speed);

        TaskStats {
            upload_id: self.upload_id,
            total_bytes: self.total_bytes,
            bytes_uploaded,
            instant_speed,
            average_speed,
            percentage,
            eta
        }
    }

    fn calculate_average_speed(&self, bytes_uploaded: u64) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            bytes_uploaded as f64 / elapsed
        } else {
            0.0
        }
    }

    fn calculate_percentage(&self, bytes_uploaded: u64) -> f64 {
        if self.total_bytes > 0 {
            (bytes_uploaded as f64 / self.total_bytes as f64) * 100.0
        } else {
            0.0
        }
    }

    fn calculate_eta(&self, bytes_uploaded: u64, speed: f64) -> Option<Duration> {
        let remaining = self.total_bytes.saturating_sub(bytes_uploaded);
        if remaining == 0 {
            return Some(Duration::from_secs(0))
        }

        if speed > 0.0 {
            Some(Duration::from_secs_f64(remaining as f64 / speed))
        } else {
            None
        }
    }
}

/// 速度计算器（使用环形缓冲区）
struct SpeedCalculator {
    samples: Vec<SpeedSample>,
    write_index: usize,
    sample_count: usize,
    max_samples: usize,
    // 添加速度平滑
    smoothed_speed: f64,
    smoothing_factor: f64,
    // 记录开始时间，用于初始阶段的速度计算
    start_time: Instant,
    first_sample_time: Option<Instant>,
}

/// 样本
#[derive(Clone, Copy, Debug)]
struct SpeedSample {
    bytes_total: u64,
    timestamp: Instant,
}

impl SpeedCalculator {
    fn new() -> Self {
        let max_samples = 30;  // 增加样本数量以获得更稳定的速度
        let now = Instant::now();
        Self {
            samples: vec![SpeedSample {
                bytes_total: 0,
                timestamp: now,
            }; max_samples],
            write_index: 0,
            sample_count: 0,
            max_samples,
            smoothed_speed: 0.0,
            smoothing_factor: 0.15,  // EMA 平滑因子
            start_time: now,
            first_sample_time: None,
        }
    }

    fn add_sample(&mut self, bytes_total: u64, timestamp: Instant) {
        // 记录第一个真实样本的时间
        if self.first_sample_time.is_none() && bytes_total > 0 {
            self.first_sample_time = Some(timestamp);
        }

        // 忽略太频繁的更新（小于50ms）
        if self.sample_count > 0 {
            let last_idx = (self.write_index + self.max_samples - 1) % self.max_samples;
            let time_since_last = timestamp.duration_since(self.samples[last_idx].timestamp);
            if time_since_last.as_millis() < 50 {
                return;
            }
        }

        // 存储新样本
        self.samples[self.write_index] = SpeedSample {
            bytes_total,
            timestamp,
        };

        self.write_index = (self.write_index + 1) % self.max_samples;
        self.sample_count = self.sample_count.saturating_add(1).min(self.max_samples);
    }

    fn get_instant_speed(&mut self) -> f64 {
        if self.sample_count < 2 {
            return 0.0;
        }

        // 在初始阶段（前10秒），使用从开始到现在的平均速度
        if let Some(first_time) = self.first_sample_time {
            let elapsed_since_start = Instant::now().duration_since(first_time);
            if elapsed_since_start < Duration::from_secs(10) {
                // 使用从开始到现在的总体平均速度
                let newest_idx = (self.write_index + self.max_samples - 1) % self.max_samples;
                let newest = &self.samples[newest_idx];
                let first_idx = (self.write_index + self.max_samples - self.sample_count) % self.max_samples;
                let first = &self.samples[first_idx];
                
                let total_bytes = newest.bytes_total;
                let total_time = newest.timestamp.duration_since(first_time).as_secs_f64();
                
                if total_time > 0.5 {  // 至少0.5秒的数据
                    let avg_speed = total_bytes as f64 / total_time;
                    // 在初始阶段使用更大的平滑因子
                    self.smoothed_speed = if self.smoothed_speed == 0.0 {
                        avg_speed
                    } else {
                        self.smoothed_speed * 0.7 + avg_speed * 0.3
                    };
                    return self.smoothed_speed;
                }
            }
        }

        // 使用较长的时间窗口（至少1秒）来计算速度
        let min_time_window = Duration::from_secs(1);
        let max_time_window = Duration::from_secs(5);
        
        let newest_idx = (self.write_index + self.max_samples - 1) % self.max_samples;
        let newest = &self.samples[newest_idx];
        
        // 找到合适的时间窗口内的最旧样本
        let mut oldest_idx = newest_idx;
        let mut samples_checked = 0;
        
        for i in 1..self.sample_count {
            let idx = (self.write_index + self.max_samples - i - 1) % self.max_samples;
            let sample = &self.samples[idx];
            
            let time_diff = newest.timestamp.duration_since(sample.timestamp);
            
            // 如果时间差超过最大窗口，停止
            if time_diff > max_time_window {
                break;
            }
            
            oldest_idx = idx;
            samples_checked += 1;
            
            // 如果时间差至少达到最小窗口，可以使用
            if time_diff >= min_time_window {
                break;
            }
        }
        
        // 如果没有足够的历史数据，返回当前平滑速度
        if samples_checked == 0 {
            return self.smoothed_speed;
        }
        
        let oldest = &self.samples[oldest_idx];
        
        // 计算时间差
        let time_diff = newest.timestamp.duration_since(oldest.timestamp).as_secs_f64();
        if time_diff < 0.1 {  // 至少100ms
            return self.smoothed_speed;
        }
        
        // 计算字节差
        let bytes_diff = newest.bytes_total.saturating_sub(oldest.bytes_total);
        
        // 计算原始速度
        let raw_speed = bytes_diff as f64 / time_diff;
        
        // 应用指数移动平均（EMA）平滑
        self.smoothed_speed = if self.smoothed_speed == 0.0 {
            raw_speed
        } else {
            self.smoothed_speed * (1.0 - self.smoothing_factor) + raw_speed * self.smoothing_factor
        };
        
        self.smoothed_speed
    }
}

/// 任务状态
struct TaskStats {
    upload_id: UploadId,
    total_bytes: u64,
    bytes_uploaded: u64,
    instant_speed: f64,
    average_speed: f64,
    percentage: f64,
    eta: Option<Duration>,
}

/// 进度聚合器句柄
pub struct ProgressAggregatorHandle {
    pub aggregator: Arc<ProgressAggregator>,
    collector_handle: tokio::task::JoinHandle<()>,
    reporter_handle: tokio::task::JoinHandle<()>,
}

impl ProgressAggregatorHandle {
    pub async fn shutdown(self) {
        // 取消所有任务
        self.aggregator.cancellation_token.cancel();
        
        // 等待任务完成
        let _ = tokio::join!(
            self.collector_handle,
            self.reporter_handle
        );
    }
}
