use futures_util::future::TryFutureExt;
use graphgate_core::{Request, Response};
use reqwest::Error;

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
    type Error = Error;

    async fn is_ready(&self) -> bool {
        true
    }

    async fn query(&self, request: Request) -> Result<Response, Self::Error> {
        self.client
            .post(&self.url)
            .json(&request)
            .send()
            .and_then(|resp| resp.json::<Response>())
            .await
    }
}
