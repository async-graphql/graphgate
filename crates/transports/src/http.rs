use futures_util::future::TryFutureExt;
use graphgate_core::Response;
use reqwest::Error;
use value::{value, Variables};

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

    async fn query(&self, query: &str, variables: Variables) -> Result<Response, Self::Error> {
        self.client
            .post(&self.url)
            .json(&value!({ "query": query, "variables": variables }))
            .send()
            .and_then(|resp| resp.json::<Response>())
            .await
    }
}
