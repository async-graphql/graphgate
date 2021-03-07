use anyhow::{Context, Error, Result};
use url::Url;

use crate::http::HttpTransport;
use crate::transport::Transport;
use crate::websocket::WebSocketTransport;
use crate::wrapper::TransportWrapper;

pub fn create_transport(url: impl AsRef<str>) -> Result<Box<dyn Transport<Error = Error>>> {
    let parsed_url =
        Url::parse(url.as_ref()).context(format!("Failed to parse url: {}", url.as_ref()))?;
    match parsed_url.scheme() {
        "http" | "https" => Ok(Box::new(TransportWrapper(HttpTransport::new(url.as_ref())))),
        "ws" | "wss" => Ok(Box::new(TransportWrapper(WebSocketTransport::new(
            url.as_ref(),
        )))),
        _ => anyhow::bail!("Unknown scheme: {}", parsed_url.scheme()),
    }
}
