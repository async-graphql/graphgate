use anyhow::Result;
use futures_util::future::TryFutureExt;
use futures_util::stream::BoxStream;
use graphgate_core::{Request, Response};

use crate::transport::Transport;

pub struct HttpTransport {
    client: reqwest::Client,
    url: String,
}

impl HttpTransport {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: url.into(),
        }
    }
}

#[async_trait::async_trait]
impl Transport for HttpTransport {
    async fn query(&self, request: Request) -> Result<Response> {
        Ok(self
            .client
            .post(&self.url)
            .json(&request)
            .send()
            .and_then(|resp| resp.json::<Response>())
            .await?)
    }

    async fn subscribe(&self, _request: Request) -> Result<BoxStream<'static, Response>> {
        anyhow::bail!("Not supported.")
    }
}
