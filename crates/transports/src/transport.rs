use std::fmt::Display;

use graphgate_core::{Request, Response};

#[async_trait::async_trait]
pub trait Transport: Sync + Send + 'static {
    type Error: Display + 'static;

    async fn is_ready(&self) -> bool;

    async fn query(&self, request: Request) -> Result<Response, Self::Error>;
}
