use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use std::collections::VecDeque;

/// 速度计算器
pub struct SpeedCalculator {
    /// 历史记录
    history: Mutex<VecDeque<(Instant, u64)>>,
    /// 历史窗口大小
    window_size: Duration,
    /// 最大历史记录数
    max_entries: usize,
}

impl SpeedCalculator {
    pub fn new(window_size: Duration) -> Self {
        Self {
            history: Mutex::new(VecDeque::new()),
            window_size,
            max_entries: 100,
        }
    }
    
    /// 添加数据点
    pub async fn add_data_point(&self, bytes: u64) {
        let now = Instant::now();
        let mut history = self.history.lock().await;
        
        // 添加新数据点
        history.push_back((now, bytes));
        
        // 移除过期的数据点
        let cutoff = now - self.window_size;
        while let Some(&(time, _)) = history.front() {
            if time < cutoff {
                history.pop_front();
            } else {
                break;
            }
        }
        
        // 限制历史记录数量
        while history.len() > self.max_entries {
            history.pop_front();
        }
    }
    
    /// 计算当前速度（字节/秒）
    pub async fn calculate_speed(&self) -> f64 {
        let history = self.history.lock().await;
        
        if history.len() < 2 {
            return 0.0;
        }
        
        let first = history.front().unwrap();
        let last = history.back().unwrap();
        
        let duration = last.0.duration_since(first.0).as_secs_f64();
        let bytes = last.1 - first.1;
        
        if duration > 0.0 {
            bytes as f64 / duration
        } else {
            0.0
        }
    }
    
    /// 计算平均速度
    pub async fn calculate_average_speed(&self, total_bytes: u64, start_time: Instant) -> f64 {
        let duration = start_time.elapsed().as_secs_f64();
        if duration > 0.0 {
            total_bytes as f64 / duration
        } else {
            0.0
        }
    }
    
    /// 估算剩余时间
    pub async fn estimate_eta(&self, remaining_bytes: u64) -> Option<Duration> {
        let speed = self.calculate_speed().await;
        
        if speed > 0.0 {
            let seconds = remaining_bytes as f64 / speed;
            Some(Duration::from_secs_f64(seconds))
        } else {
            None
        }
    }
}

/// 进度跟踪器
pub struct ProgressTracker {
    /// 总字节数
    total_bytes: u64,
    /// 已传输字节数
    transferred_bytes: Mutex<u64>,
    /// 开始时间
    start_time: Instant,
    /// 速度计算器
    speed_calculator: SpeedCalculator,
}

impl ProgressTracker {
    pub fn new(total_bytes: u64) -> Self {
        Self {
            total_bytes,
            transferred_bytes: Mutex::new(0),
            start_time: Instant::now(),
            speed_calculator: SpeedCalculator::new(Duration::from_secs(5)),
        }
    }
    
    /// 更新进度
    pub async fn update(&self, bytes: u64) {
        let mut transferred = self.transferred_bytes.lock().await;
        *transferred = bytes;
        drop(transferred);
        
        self.speed_calculator.add_data_point(bytes).await;
    }
    
    /// 增加传输字节数
    pub async fn add_bytes(&self, bytes: u64) {
        let mut transferred = self.transferred_bytes.lock().await;
        *transferred += bytes;
        let total = *transferred;
        drop(transferred);
        
        self.speed_calculator.add_data_point(total).await;
    }
    
    /// 获取进度信息
    pub async fn get_progress(&self) -> crate::core::UploadProgress {
        let transferred = *self.transferred_bytes.lock().await;
        let speed = self.speed_calculator.calculate_speed().await;
        let average_speed = self.speed_calculator.calculate_average_speed(transferred, self.start_time).await;
        let percentage = if self.total_bytes > 0 {
            (transferred as f64 / self.total_bytes as f64) * 100.0
        } else {
            0.0
        };
        
        let remaining = self.total_bytes.saturating_sub(transferred);
        let eta = self.speed_calculator.estimate_eta(remaining).await;
        
        crate::core::UploadProgress {
            uploaded_bytes: transferred,
            total_bytes: self.total_bytes,
            speed,
            average_speed,
            percentage,
            eta,
        }
    }
    
    /// 获取已传输字节数
    pub async fn get_transferred_bytes(&self) -> u64 {
        *self.transferred_bytes.lock().await
    }
    
    /// 获取运行时间
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }
}

/// 格式化字节数
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    const UNIT_SIZE: f64 = 1024.0;
    
    let mut size = bytes as f64;
    let mut unit_index = 0;
    
    while size >= UNIT_SIZE && unit_index < UNITS.len() - 1 {
        size /= UNIT_SIZE;
        unit_index += 1;
    }
    
    format!("{:.2} {}", size, UNITS[unit_index])
}

/// 格式化速度
pub fn format_speed(bytes_per_second: f64) -> String {
    format!("{}/s", format_bytes(bytes_per_second as u64))
}

/// 格式化持续时间
pub fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    
    if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}
