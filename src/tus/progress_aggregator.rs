use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use parking_lot::RwLock;
use tokio::sync::mpsc;
use tokio::time;
use crate::tus::types::{AggregatedStats, BatchProgress, TaskProgress, UploadId};

/// 进度聚合器
pub struct ProgressAggregator {
    /// 所有任务进度信息
    tasks: Arc<RwLock<HashMap<UploadId, TaskProgressTracker>>>,

    /// 批量进度更新发送器
    batch_tx: mpsc::UnboundedSender<BatchProgress>,

    /// 更新间隔
    update_interval: Duration,

    /// 是否启用聚合
    enabled: Arc<AtomicBool>,
}

impl ProgressAggregator {
    pub fn new(batch_tx: mpsc::UnboundedSender<BatchProgress>) -> Self {
        Self {
            batch_tx,
            tasks: Arc::new(RwLock::new(HashMap::new())),
            update_interval: Duration::from_secs(1),
            enabled: Arc::new(AtomicBool::new(true)),
        }
    }

    /// 设置更新间隔
    pub fn with_update_interval(mut self, update_interval: Duration) -> Self {
        self.update_interval = update_interval;
        self
    }

    /// 注册新任务
    pub fn register_task(&self, upload_id: UploadId, total_bytes: u64) {
        let mut tasks = self.tasks.write();
        tasks.insert(upload_id, TaskProgressTracker::new(upload_id, total_bytes));
    }

    /// 注销任务
    pub fn unregister_task(&self, upload_id: UploadId) {
        let mut tasks = self.tasks.write();
        tasks.remove(&upload_id);
    }

    /// 启动
    pub fn start(self) -> ProgressAggregatorHandle {
        let tasks = self.tasks.clone();
        let enabled = self.enabled.clone();
        let batch_tx = self.batch_tx.clone();
        let update_interval = self.update_interval;

        let handle = tokio::spawn(async move {
            // 创建一个定时器，按照给定 interval 周期性地触发
            let mut interval = time::interval(update_interval);
            // 指定当程序/系统运行缓慢，处理任务的时间超过了定时器间隔时的行为
            // Burst(default): 会立即触发所有错过的滴答，直到"追上"预期的时间点
            // Skip: 跳过所有错过的滴答，从当前时间重新开始计时
            // Delay: 保持原有的时间节拍，但允许整体时间线向后延迟
            interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

            loop {
                interval.tick().await;

                if !enabled.load(Ordering::Relaxed) {
                    continue;
                }

                let tasks_guard = tasks.read();
                if tasks_guard.is_empty() {
                    continue;
                }

                // 开始收集任务进度
                let mut task_progresses = Vec::new();
                let mut total_bytes = 0u64;
                let mut total_uploaded = 0u64;
                let mut total_speed = 0.0;

                // 应该要过滤掉 错误/暂停
                for tracker in tasks_guard.values() {
                    let instant_speed = tracker.instant_speed();
                    let average_speed = tracker.average_speed();
                    let percentage = tracker.percentage();
                    let eta = tracker.eta();

                    total_bytes += tracker.total_bytes;
                    total_uploaded += tracker.bytes_uploaded;
                    total_speed += instant_speed;

                    task_progresses.push(TaskProgress {
                        upload_id: tracker.upload_id,
                        bytes_uploaded: tracker.bytes_uploaded,
                        total_bytes: tracker.total_bytes,
                        instant_speed,
                        average_speed,
                        percentage,
                        eta,
                    });
                }

                drop(tasks_guard);

                // 总体百分比
                let overall_percentage = if total_bytes > 0 {
                    (total_uploaded as f64) / (total_bytes as f64) * 100.0
                } else {
                    0.0
                };

                // 总体剩余时间
                let overall_eta = if total_speed > 0.0 {
                    let remaining = total_bytes - total_uploaded;
                    Some(Duration::from_secs_f64(remaining as f64 / total_speed))
                } else {
                    None
                };

                let aggregated_stats = AggregatedStats {
                    total_tasks: task_progresses.len(),
                    active_tasks: task_progresses
                        .iter()
                        .filter(|p| p.percentage < 100.0)
                        .count(),
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

                // 通知更新
                let _ = batch_tx.send(batch_progress);
            }
        });

        ProgressAggregatorHandle {
            aggregator: self,
            join_handle: handle,
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
        let mut tasks = self.tasks.write();
        if let Some(tracker) = tasks.get_mut(&upload_id) {
            tracker.update(bytes_uploaded);
        }
    }

    /// 获取当前统计信息（同步）
    pub fn get_current_stats(&self) -> Option<AggregatedStats> {
        let tasks = self.tasks.read();
        if tasks.is_empty() {
            return None;
        }

        let mut total_bytes = 0u64;
        let mut total_uploaded = 0u64;
        let mut total_speed = 0.0;
        let mut active_count = 0;

        for tracker in tasks.values() {
            total_bytes += tracker.total_bytes;
            total_uploaded += tracker.bytes_uploaded;
            total_speed += tracker.instant_speed();

            if tracker.percentage() < 100.0 {
                active_count += 1;
            }
        }

        let overall_percentage = if total_bytes > 0 {
            (total_uploaded as f64) / (total_bytes as f64) * 100.0
        } else {
            0.0
        };

        let overall_eta = if total_speed > 0.0 {
            let remaining = total_bytes - total_uploaded;
            Some(Duration::from_secs_f64(remaining as f64 / total_speed))
        } else {
            None
        };

        Some(AggregatedStats {
            total_tasks: tasks.len(),
            active_tasks: active_count,
            total_bytes,
            total_uploaded,
            overall_speed: total_speed,
            overall_percentage,
            overall_eta,
        })
    }
}

/// 单个任务进度跟踪器
pub struct TaskProgressTracker {
    /// 任务 id
    upload_id: UploadId,

    /// 文件总大小
    total_bytes: u64,

    /// 已上传字节
    bytes_uploaded: u64,

    /// 上次更新时的字节数
    last_bytes: u64,

    /// 上次更新时间
    last_update: Instant,

    /// 开始时间
    start_time: Instant,

    /// 速度样本（用于计算瞬时速度）
    speed_samples: Vec<(Instant, u64)>,

    /// 最大样本数量
    max_samples: usize,
}

impl TaskProgressTracker {
    pub fn new(upload_id: UploadId, total_bytes: u64) -> Self {
        Self {
            upload_id,
            total_bytes,
            bytes_uploaded: 0,
            last_bytes: 0,
            last_update: Instant::now(),
            start_time: Instant::now(),
            speed_samples: Vec::with_capacity(10),
            max_samples: 10,
        }
    }

    /// 更新进度
    fn update(&mut self, bytes_uploaded: u64) {
        let now = Instant::now();
        // 这次传输的字节
        let bytes_diff = bytes_uploaded.saturating_sub(self.last_bytes);

        if bytes_diff > 0 {
            // 添加样本
            if self.speed_samples.len() >= self.max_samples {
                self.speed_samples.remove(0);
            }

            self.speed_samples.push((now, bytes_diff));

            self.last_bytes = bytes_uploaded;
            self.bytes_uploaded = bytes_uploaded;
            self.last_update = now;
        }
    }

    /// 计算瞬时速度
    fn instant_speed(&self) -> f64 {
        if self.speed_samples.len() < 2 {
            return 0.0;
        }

        let recent_count = (self.speed_samples.len() / 3).max(2);
        let recent_samples = &self.speed_samples[self.speed_samples.len() - recent_count..];

        let total_bytes: u64 = recent_samples.iter().map(|(_, size)| size).sum();
        let duration = recent_samples.last().unwrap().0
            .duration_since(recent_samples.first().unwrap().0)
            .as_secs_f64();

        if duration > 0.0 {
            total_bytes as f64 / duration
        } else {
            0.0
        }
    }

    /// 计算平均速度
    fn average_speed(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.bytes_uploaded as f64 / elapsed
        } else {
            0.0
        }
    }

    /// 进度百分比
    fn percentage(&self) -> f64 {
        if self.total_bytes > 0 {
            (self.bytes_uploaded as f64 / self.total_bytes as f64) * 100.0
        } else {
            0.0
        }
    }

    /// 计算剩余时间
    fn eta(&self) -> Option<Duration> {
        let remaining = self.total_bytes.saturating_sub(self.bytes_uploaded);
        if remaining == 0 {
            return Some(Duration::from_secs(0));
        }

        let speed = self.instant_speed();
        if speed > 0.0 {
            Some(Duration::from_secs_f64(remaining as f64 / speed))
        } else {
            None
        }
    }
}

/// 进度聚合器句柄
pub struct ProgressAggregatorHandle {
    pub aggregator: ProgressAggregator,
    join_handle: tokio::task::JoinHandle<()>,
}

impl ProgressAggregatorHandle {
    /// 关闭
    pub async fn shutdown(self) {
        self.aggregator.pause();
        self.join_handle.abort();
        let _ = self.join_handle.await;
    }
}

/// 创建单任务进度回调
pub fn create_task_progress_callback(aggregator: Arc<ProgressAggregator>, upload_id: UploadId)
    -> Arc<dyn Fn(u64) + Send + Sync>
{
    Arc::new(move |bytes_uploaded| {
        aggregator.update_task_progress(upload_id, bytes_uploaded);
    })
}
