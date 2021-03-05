use std::collections::HashMap;

use anyhow::{Context, Error, Result};
use graphgate_core::{Coordinator, Request, Response};
use url::Url;

use crate::http::HttpTransport;
use crate::transport::Transport;
use crate::websocket::WebSocketTransport;
use crate::wrapper::TransportWrapper;

#[derive(Default)]
pub struct CoordinatorImpl(HashMap<String, Box<dyn Transport<Error = Error>>>);

#[async_trait::async_trait]
impl Coordinator for CoordinatorImpl {
    type Error = Error;

    async fn query(&self, service: &str, request: Request) -> Result<Response, Self::Error> {
        match self.0.get(service) {
            Some(transport) => transport.query(request).await,
            None => anyhow::bail!("Service '{}' is not defined."),
        }
    }
}

impl CoordinatorImpl {
    pub fn add(mut self, service: impl Into<String>, transport: impl Transport) -> Self {
        self.0
            .insert(service.into(), Box::new(TransportWrapper(transport)));
        self
    }

    pub fn add_url(self, service: impl Into<String>, url: impl AsRef<str>) -> Result<Self> {
        let parsed_url =
            Url::parse(url.as_ref()).context(format!("Failed to parse url: {}", url.as_ref()))?;
        match parsed_url.scheme() {
            "http" | "https" => Ok(self.add(service, HttpTransport::new(url.as_ref()))),
            "ws" | "wss" => Ok(self.add(service, WebSocketTransport::new(url.as_ref()))),
            _ => anyhow::bail!("Unknown scheme: {}", parsed_url.scheme()),
        }
    }
}
