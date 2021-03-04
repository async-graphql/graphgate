use std::fmt::Display;

use graphgate_core::Response;
use value::Variables;

#[async_trait::async_trait]
pub trait Transport: Sync + Send + 'static {
    type Error: Display + 'static;

    async fn query(&self, query: &str, variables: Variables) -> Result<Response, Self::Error>;
}
