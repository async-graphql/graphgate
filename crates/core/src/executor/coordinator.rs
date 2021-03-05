use std::fmt::Display;
use std::sync::Arc;

use crate::{Request, Response};

#[async_trait::async_trait]
pub trait Coordinator: Sync + Send {
    type Error: Display + 'static;

    async fn query(&self, service: &str, request: Request) -> Result<Response, Self::Error>;
}

#[async_trait::async_trait]
impl<T: Coordinator> Coordinator for Arc<T> {
    type Error = T::Error;

    async fn query(&self, service: &str, request: Request) -> Result<Response, Self::Error> {
        self.as_ref().query(service, request).await
    }
}
