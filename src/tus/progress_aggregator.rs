use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use crate::tus::types::{AggregatedStats, BatchProgress, TaskProgress, UploadId};

/// 进度聚合器 - 优化版本
pub struct ProgressAggregator {
    /// 命令发送通道
    command_tx: mpsc::Sender<AggregatorCommand>,
    
    /// 活跃任务计数
    active_task_count: Arc<AtomicUsize>,
    
    /// 取消令牌
    cancellation_token: CancellationToken,
}

/// 聚合器命令
enum AggregatorCommand {
    /// 注册任务
    RegisterTask {
        upload_id: UploadId,
        total_bytes: u64,
    },
    
    /// 注销任务
    UnregisterTask {
        upload_id: UploadId,
    },
    
    /// 更新进度
    UpdateProgress {
        upload_id: UploadId,
        bytes_uploaded: u64,
        timestamp: Instant,
    },
    
    /// 设置是否启用
    SetEnabled(bool),
}

/// 任务跟踪器
struct TaskTracker {
    upload_id: UploadId,
    total_bytes: u64,
    bytes_uploaded: u64,
    start_time: Instant,
    last_update_time: Instant,
    speed_calculator: SpeedCalculator,
}

impl TaskTracker {
    fn new(upload_id: UploadId, total_bytes: u64) -> Self {
        let now = Instant::now();
        Self {
            upload_id,
            total_bytes,
            bytes_uploaded: 0,
            start_time: now,
            last_update_time: now,
            speed_calculator: SpeedCalculator::new(),
        }
    }
    
    fn update(&mut self, bytes_uploaded: u64, timestamp: Instant) {
        self.bytes_uploaded = bytes_uploaded;
        self.last_update_time = timestamp;
        self.speed_calculator.add_sample(bytes_uploaded, timestamp);
    }
    
    fn get_progress(&mut self) -> TaskProgress {
        let instant_speed = self.speed_calculator.get_instant_speed();
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let average_speed = if elapsed > 0.0 {
            self.bytes_uploaded as f64 / elapsed
        } else {
            0.0
        };
        
        let percentage = if self.total_bytes > 0 {
            (self.bytes_uploaded as f64 / self.total_bytes as f64) * 100.0
        } else {
            0.0
        };
        
        let remaining = self.total_bytes.saturating_sub(self.bytes_uploaded);
        let eta = if instant_speed > 0.0 && remaining > 0 {
            Some(Duration::from_secs_f64(remaining as f64 / instant_speed))
        } else {
            None
        };
        
        TaskProgress {
            upload_id: self.upload_id,
            bytes_uploaded: self.bytes_uploaded,
            total_bytes: self.total_bytes,
            instant_speed,
            average_speed,
            percentage,
            eta,
        }
    }
}

/// 聚合器工作器 - 单一任务处理所有逻辑
struct AggregatorWorker {
    /// 任务注册表
    tasks: HashMap<UploadId, TaskTracker>,
    
    /// 批量进度发送器
    batch_tx: mpsc::UnboundedSender<BatchProgress>,
    
    /// 命令接收器
    command_rx: mpsc::Receiver<AggregatorCommand>,
    
    /// 更新间隔
    update_interval: Duration,
    
    /// 是否启用
    enabled: bool,
    
    /// 活跃任务计数
    active_task_count: Arc<AtomicUsize>,
}

impl AggregatorWorker {
    async fn run(mut self, cancellation_token: CancellationToken) {
        let mut report_interval = interval(self.update_interval);
        report_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        
        loop {
            tokio::select! {
                // 处理取消
                _ = cancellation_token.cancelled() => {
                    break;
                }
                
                // 处理命令
                Some(command) = self.command_rx.recv() => {
                    self.handle_command(command);
                }
                
                // 定时报告进度
                _ = report_interval.tick() => {
                    if self.enabled && !self.tasks.is_empty() {
                        self.report_progress();
                    }
                }
            }
        }
    }
    
    fn handle_command(&mut self, command: AggregatorCommand) {
        match command {
            AggregatorCommand::RegisterTask { upload_id, total_bytes } => {
                self.tasks.insert(upload_id, TaskTracker::new(upload_id, total_bytes));
                self.active_task_count.fetch_add(1, Ordering::SeqCst);
            }
            
            AggregatorCommand::UnregisterTask { upload_id } => {
                if self.tasks.remove(&upload_id).is_some() {
                    self.active_task_count.fetch_sub(1, Ordering::SeqCst);
                }
            }
            
            AggregatorCommand::UpdateProgress { upload_id, bytes_uploaded, timestamp } => {
                if let Some(tracker) = self.tasks.get_mut(&upload_id) {
                    tracker.update(bytes_uploaded, timestamp);
                }
            }
            
            AggregatorCommand::SetEnabled(enabled) => {
                self.enabled = enabled;
            }
        }
    }
    
    fn report_progress(&mut self) {
        let mut task_progresses = Vec::with_capacity(self.tasks.len());
        let mut total_bytes = 0u64;
        let mut total_uploaded = 0u64;
        let mut total_speed = 0.0;
        let mut active_count = 0;
        
        // 收集所有任务的进度
        for tracker in self.tasks.values_mut() {
            let progress = tracker.get_progress();
            
            total_bytes += progress.total_bytes;
            total_uploaded += progress.bytes_uploaded;
            total_speed += progress.instant_speed;
            
            if progress.percentage < 100.0 {
                active_count += 1;
            }
            
            task_progresses.push(progress);
        }
        
        // 计算聚合统计
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
        
        // 发送批量进度
        let _ = self.batch_tx.send(batch_progress);
    }
}

impl ProgressAggregator {
    pub fn new(batch_tx: mpsc::UnboundedSender<BatchProgress>) -> Self {
        let (command_tx, command_rx) = mpsc::channel(1000);
        let active_task_count = Arc::new(AtomicUsize::new(0));
        let cancellation_token = CancellationToken::new();
        
        // 创建工作器
        let worker = AggregatorWorker {
            tasks: HashMap::new(),
            batch_tx,
            command_rx,
            update_interval: Duration::from_secs(1),
            enabled: true,
            active_task_count: active_task_count.clone(),
        };
        
        // 启动单一工作任务
        let token_clone = cancellation_token.clone();
        tokio::spawn(async move {
            worker.run(token_clone).await;
        });
        
        Self {
            command_tx,
            active_task_count,
            cancellation_token,
        }
    }
    
    /// 设置更新间隔
    pub fn with_update_interval(self, _interval: Duration) -> Self {
        // 注意：这个方法现在需要通过命令来实现
        self
    }
    
    /// 注册任务
    pub fn register_task(&self, upload_id: UploadId, total_bytes: u64) {
        let _ = self.command_tx.try_send(AggregatorCommand::RegisterTask {
            upload_id,
            total_bytes,
        });
    }
    
    /// 注销任务
    pub fn unregister_task(&self, upload_id: UploadId) {
        let _ = self.command_tx.try_send(AggregatorCommand::UnregisterTask {
            upload_id,
        });
    }
    
    /// 更新任务进度
    pub fn update_task_progress(&self, upload_id: UploadId, bytes_uploaded: u64) {
        let _ = self.command_tx.try_send(AggregatorCommand::UpdateProgress {
            upload_id,
            bytes_uploaded,
            timestamp: Instant::now(),
        });
    }
    
    /// 暂停
    pub fn pause(&self) {
        let _ = self.command_tx.try_send(AggregatorCommand::SetEnabled(false));
    }
    
    /// 恢复
    pub fn resume(&self) {
        let _ = self.command_tx.try_send(AggregatorCommand::SetEnabled(true));
    }
    
    /// 获取活跃任务数
    pub fn active_task_count(&self) -> usize {
        self.active_task_count.load(Ordering::Relaxed)
    }
    
    /// 启动（为了兼容性保留，但实际在 new 中已启动）
    pub fn start(self: Arc<Self>) -> ProgressAggregatorHandle {
        ProgressAggregatorHandle {
            aggregator: self,
        }
    }
}

/// 速度计算器 - 简化版本
struct SpeedCalculator {
    samples: Vec<(u64, Instant)>,  // (bytes_total, timestamp)
    max_samples: usize,
    smoothed_speed: f64,
    first_sample_time: Option<Instant>,
}

impl SpeedCalculator {
    fn new() -> Self {
        Self {
            samples: Vec::with_capacity(30),
            max_samples: 30,
            smoothed_speed: 0.0,
            first_sample_time: None,
        }
    }
    
    fn add_sample(&mut self, bytes_total: u64, timestamp: Instant) {
        // 记录第一个样本时间
        if self.first_sample_time.is_none() && bytes_total > 0 {
            self.first_sample_time = Some(timestamp);
        }
        
        // 添加样本
        self.samples.push((bytes_total, timestamp));
        
        // 保持样本数量限制
        if self.samples.len() > self.max_samples {
            self.samples.remove(0);
        }
    }
    
    fn get_instant_speed(&mut self) -> f64 {
        if self.samples.len() < 2 {
            return 0.0;
        }
        
        let now = Instant::now();
        
        // 初始阶段特殊处理
        if let Some(first_time) = self.first_sample_time {
            let elapsed = now.duration_since(first_time);
            if elapsed < Duration::from_secs(10) {
                // 使用总体平均速度
                let (last_bytes, _) = self.samples.last().unwrap();
                let avg_speed = *last_bytes as f64 / elapsed.as_secs_f64();
                self.smoothed_speed = self.smoothed_speed * 0.7 + avg_speed * 0.3;
                return self.smoothed_speed;
            }
        }
        
        // 找到1-5秒窗口内的样本
        let min_window = Duration::from_secs(1);
        let max_window = Duration::from_secs(5);
        
        let (newest_bytes, newest_time) = self.samples.last().unwrap();
        let mut oldest_idx = self.samples.len() - 1;
        
        for i in (0..self.samples.len() - 1).rev() {
            let (_, time) = &self.samples[i];
            let diff = newest_time.duration_since(*time);
            
            if diff > max_window {
                break;
            }
            
            oldest_idx = i;
            
            if diff >= min_window {
                break;
            }
        }
        
        let (oldest_bytes, oldest_time) = &self.samples[oldest_idx];
        let time_diff = newest_time.duration_since(*oldest_time).as_secs_f64();
        
        if time_diff > 0.1 {
            let bytes_diff = newest_bytes.saturating_sub(*oldest_bytes);
            let raw_speed = bytes_diff as f64 / time_diff;
            self.smoothed_speed = self.smoothed_speed * 0.85 + raw_speed * 0.15;
        }
        
        self.smoothed_speed
    }
}

/// 进度聚合器句柄
pub struct ProgressAggregatorHandle {
    pub aggregator: Arc<ProgressAggregator>,
}

impl ProgressAggregatorHandle {
    pub async fn shutdown(self) {
        self.aggregator.cancellation_token.cancel();
        // 给一些时间让任务清理
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
