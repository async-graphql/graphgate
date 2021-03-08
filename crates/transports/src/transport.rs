use anyhow::Result;
use futures_util::stream::BoxStream;
use graphgate_core::{Request, Response};

#[async_trait::async_trait]
pub trait Transport: Sync + Send + 'static {
    async fn query(&self, request: Request) -> Result<Response>;

    fn is_support_subscribe(&self) -> bool {
        false
    }

    async fn subscribe(&self, request: Request) -> Result<BoxStream<'static, Response>>;
}
