use anyhow::{Error, Result};
use graphgate_core::{Request, Response};

use super::wrapper::TransportWrapper;
use crate::transport::Transport;
use crate::utils::create_transport;

#[derive(Default)]
pub struct RoundRobinTransport {
    transports: Vec<Box<dyn Transport<Error = Error>>>,
}

impl RoundRobinTransport {
    pub fn add<T>(mut self, transport: impl Transport) -> Self {
        self.transports.push(Box::new(TransportWrapper(transport)));
        self
    }

    pub fn add_url(mut self, url: impl AsRef<str>) -> Result<Self> {
        self.transports.push(create_transport(url)?);
        Ok(self)
    }
}

#[async_trait::async_trait]
impl Transport for RoundRobinTransport {
    type Error = Error;

    async fn is_ready(&self) -> bool {
        true
    }

    async fn query(&self, request: Request) -> Result<Response, Self::Error> {
        let mut transports = Vec::with_capacity(self.transports.len());
        for transport in &self.transports {
            if transport.is_ready().await {
                transports.push(transport.as_ref());
            }
        }
        anyhow::ensure!(!transports.is_empty(), "Not ready.");
        transports[fastrand::usize(0..transports.len())]
            .query(request)
            .await
    }
}
