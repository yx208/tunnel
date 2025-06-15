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
        let calculator = self.speed_calculator.lock();

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
}

/// 样本
#[derive(Clone, Copy, Debug)]
struct SpeedSample {
    bytes_total: u64,
    timestamp: Instant,
}

impl SpeedCalculator {
    fn new() -> Self {
        let max_samples = 20;
        Self {
            samples: vec![SpeedSample {
                bytes_total: 0,
                timestamp: Instant::now(),
            }; max_samples],
            write_index: 0,
            sample_count: 0,
            max_samples,
        }
    }

    fn add_sample(&mut self, bytes_total: u64, timestamp: Instant) {
        // 存储新样本
        self.samples[self.write_index] = SpeedSample {
            bytes_total,
            timestamp,
        };

        self.write_index = (self.write_index + 1) % self.max_samples;
        self.sample_count = self.sample_count.saturating_add(1).min(self.max_samples);
    }

    fn get_instant_speed(&self) -> f64 {
        if self.sample_count < 2 {
            return 0.0;
        }

        // 使用最近的样本计算瞬时速度
        let window_size = (self.sample_count / 3).max(2).min(5);

        // 获取最新和最旧的样本索引
        let newest_idx = (self.write_index + self.max_samples - 1) % self.max_samples;
        let oldest_idx = (self.write_index + self.max_samples - window_size) % self.max_samples;

        let newest = &self.samples[newest_idx];
        let oldest = &self.samples[oldest_idx];
        
        // 如果时间戳相同，返回0
        if newest.timestamp <= oldest.timestamp {
            return 0.0;
        }
        
        // 计算字节差（处理可能的回退情况）
        let bytes_diff = if newest.bytes_total >= oldest.bytes_total {
            newest.bytes_total - oldest.bytes_total
        } else {
            // 如果出现字节数回退（不应该发生），返回0
            return 0.0;
        };
        
        let time_diff = newest.timestamp.duration_since(oldest.timestamp).as_secs_f64();
        
        if time_diff > 0.0 {
            bytes_diff as f64 / time_diff
        } else {
            0.0
        }
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
