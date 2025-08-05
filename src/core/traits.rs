use async_trait::async_trait;
use super::types::ProtocolType;
use super::types::TransferId;
use super::errors::Result;

#[derive(Debug, Clone)]
pub struct TransferContext {
    pub id: TransferId,
    pub protocol: ProtocolType,
}

#[async_trait]
pub trait TransferProtocol: Send + Sync {
    /// 当任务触发开始的时候做的操作
    /// 可以做本地文件检查或者服务信息检验
    async fn initialize(&self, ctx: &mut TransferContext) -> Result<()>;

    /// 当取消任务的时候会调用这个方法进行处理
    /// 如果使用 Tus/普通上传 应当销毁服务资源
    /// 如果是下载则应当销毁本地的文件
    async fn cancel(&self, ctx: &mut TransferContext) -> Result<()>;

    /// 传输完成后调用的方法
    /// 如果是上传应可以知服务
    /// 如果是下载则可以进行文件 Merge/HashCheck 之类的操作
    async fn finalize(&self, ctx: &mut TransferContext) -> Result<()>;
}

pub trait TransferTaskBuilder: Send + Sync {
    fn build_protocol(&self) -> Box<dyn TransferProtocol>;
    fn build_context(&self) -> TransferContext;
}
