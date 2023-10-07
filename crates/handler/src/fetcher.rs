use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use graphgate_planner::{Request, Response};
use http::HeaderMap;
use tokio::sync::mpsc;

use crate::{websocket::WebSocketController, ServiceRouteTable};

#[async_trait::async_trait]
pub trait Fetcher: Send + Sync {
    async fn query(&self, service: &str, request: Request) -> Result<Response>;
}

pub struct HttpFetcher<'a> {
    router_table: &'a ServiceRouteTable,
    header_map: &'a HeaderMap,
}

impl<'a> HttpFetcher<'a> {
    pub fn new(router_table: &'a ServiceRouteTable, header_map: &'a HeaderMap) -> Self {
        Self {
            router_table,
            header_map,
        }
    }
}

#[async_trait::async_trait]
impl<'a> Fetcher for HttpFetcher<'a> {
    async fn query(&self, service: &str, request: Request) -> Result<Response> {
        self.router_table
            .query(service, request, Some(self.header_map), None)
            .await
    }
}

pub struct WebSocketFetcher {
    controller: WebSocketController,
    id: AtomicU64,
}

impl WebSocketFetcher {
    pub fn new(controller: WebSocketController) -> Self {
        Self {
            controller,
            id: Default::default(),
        }
    }
}

#[async_trait::async_trait]
impl Fetcher for WebSocketFetcher {
    async fn query(&self, service: &str, request: Request) -> Result<Response> {
        let id = self.id.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = mpsc::unbounded_channel();
        self.controller
            .subscribe(format!("__req{}", id), service, request, tx)
            .await?;
        rx.recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("Connection closed."))
    }
}
