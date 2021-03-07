use std::fmt::Display;

use crate::{Request, Response};

#[async_trait::async_trait]
pub trait Coordinator: Sync + Send {
    type Error: Display + 'static;

    fn services(&self) -> Vec<String>;

    async fn query(&self, service: &str, request: Request) -> Result<Response, Self::Error>;
}
