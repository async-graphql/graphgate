use std::collections::HashMap;

use anyhow::{Error, Result};
use graphgate_core::{Coordinator, Request, Response};

use crate::transport::Transport;
use crate::utils::create_transport;
use crate::wrapper::TransportWrapper;

#[derive(Default)]
pub struct CoordinatorImpl(HashMap<String, Box<dyn Transport<Error = Error>>>);

#[async_trait::async_trait]
impl Coordinator for CoordinatorImpl {
    type Error = Error;

    fn services(&self) -> Vec<String> {
        self.0.keys().map(ToString::to_string).collect()
    }

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

    pub fn add_url(mut self, service: impl Into<String>, url: impl AsRef<str>) -> Result<Self> {
        self.0.insert(service.into(), create_transport(url)?);
        Ok(self)
    }
}
