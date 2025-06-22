use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;
use crate::core::{Result, TransferError};

/// 重试策略
#[derive(Debug, Clone)]
pub enum RetryStrategy {
    /// 固定延迟
    Fixed(Duration),
    /// 指数退避
    Exponential {
        initial: Duration,
        multiplier: f64,
        max_delay: Duration,
    },
    /// 线性退避
    Linear {
        initial: Duration,
        increment: Duration,
        max_delay: Duration,
    },
}

impl RetryStrategy {
    /// 计算第 n 次重试的延迟
    pub fn get_delay(&self, attempt: u32) -> Duration {
        match self {
            RetryStrategy::Fixed(delay) => *delay,
            RetryStrategy::Exponential { initial, multiplier, max_delay } => {
                let delay = initial.as_secs_f64() * multiplier.powf(attempt as f64);
                let delay = Duration::from_secs_f64(delay);
                std::cmp::min(delay, *max_delay)
            }
            RetryStrategy::Linear { initial, increment, max_delay } => {
                let delay = *initial + (*increment * attempt);
                std::cmp::min(delay, *max_delay)
            }
        }
    }
}

/// 重试配置
pub struct RetryConfig {
    /// 最大重试次数
    pub max_attempts: u32,
    /// 重试策略
    pub strategy: RetryStrategy,
    /// 是否重试的判断函数
    pub should_retry: Box<dyn Fn(&TransferError) -> bool + Send + Sync>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            strategy: RetryStrategy::Exponential {
                initial: Duration::from_secs(1),
                multiplier: 2.0,
                max_delay: Duration::from_secs(60),
            },
            should_retry: Box::new(|error| {
                matches!(
                    error,
                    TransferError::Http(_) | 
                    TransferError::Timeout | 
                    TransferError::ServerError { .. }
                )
            }),
        }
    }
}

/// 执行带重试的操作
pub async fn retry_with_config<F, Fut, T>(
    config: RetryConfig,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut last_error = None;
    
    for attempt in 0..config.max_attempts {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(error) => {
                // 检查是否应该重试
                if !(config.should_retry)(&error) {
                    return Err(error);
                }
                
                last_error = Some(error);
                
                // 如果不是最后一次尝试，等待后重试
                if attempt < config.max_attempts - 1 {
                    let delay = config.strategy.get_delay(attempt);
                    sleep(delay).await;
                }
            }
        }
    }
    
    // 所有重试都失败了
    Err(last_error.unwrap_or_else(|| TransferError::RetryLimitExceeded))
}

/// 使用默认配置执行重试
pub async fn retry<F, Fut, T>(operation: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    retry_with_config(RetryConfig::default(), operation).await
}

/// 重试构建器
pub struct RetryBuilder {
    config: RetryConfig,
}

impl RetryBuilder {
    pub fn new() -> Self {
        Self {
            config: RetryConfig::default(),
        }
    }
    
    pub fn max_attempts(mut self, attempts: u32) -> Self {
        self.config.max_attempts = attempts;
        self
    }
    
    pub fn strategy(mut self, strategy: RetryStrategy) -> Self {
        self.config.strategy = strategy;
        self
    }
    
    pub fn should_retry<F>(mut self, f: F) -> Self
    where
        F: Fn(&TransferError) -> bool + Send + Sync + 'static,
    {
        self.config.should_retry = Box::new(f);
        self
    }
    
    pub async fn run<F, Fut, T>(self, operation: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        retry_with_config(self.config, operation).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_retry_success() {
        let mut count = 0;
        let result = retry(|| async {
            count += 1;
            if count < 3 {
                Err(TransferError::Timeout)
            } else {
                Ok(42)
            }
        }).await;
        
        assert_eq!(result.unwrap(), 42);
        assert_eq!(count, 3);
    }
    
    #[tokio::test]
    async fn test_retry_failure() {
        let mut count = 0;
        let result = retry(|| async {
            count += 1;
            Err::<(), _>(TransferError::Timeout)
        }).await;
        
        assert!(result.is_err());
        assert_eq!(count, 3); // 默认最大重试次数
    }
}
