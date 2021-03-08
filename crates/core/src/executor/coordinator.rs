use std::fmt::Display;

use futures_util::stream::BoxStream;

use crate::{Request, Response};

#[async_trait::async_trait]
pub trait Coordinator: Sync + Send {
    type Error: Display + Send + 'static;

    fn services(&self) -> Vec<String>;

    async fn query(&self, service: &str, request: Request) -> Result<Response, Self::Error>;

    async fn subscribe(
        &self,
        service: &str,
        request: Request,
    ) -> Result<BoxStream<'static, Response>, Self::Error>;
}
