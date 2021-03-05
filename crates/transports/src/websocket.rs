use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context as _, Error, Result};
use futures_util::future::Either;
use futures_util::stream::pending;
use futures_util::{SinkExt, StreamExt};
use graphgate_core::{Request, Response};
use http::Request as HttpRequest;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::transport::Transport;

const RECONNECT_DELAY_SECONDS: u64 = 5;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

enum Command {
    IsReady {
        reply: oneshot::Sender<bool>,
    },
    Query {
        request: Request,
        reply: oneshot::Sender<Result<Response, Arc<Error>>>,
    },
}

pub struct WebSocketTransport {
    tx: mpsc::UnboundedSender<Command>,
}

impl WebSocketTransport {
    pub fn new(url: impl Into<String>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(main_loop(rx, url.into()));
        Self { tx }
    }
}

#[async_trait::async_trait]
impl Transport for WebSocketTransport {
    type Error = Arc<Error>;

    async fn is_ready(&self) -> bool {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Command::IsReady { reply: tx }).is_err() {
            return false;
        }
        rx.await.unwrap_or_default()
    }

    async fn query(&self, request: Request) -> Result<Response, Self::Error> {
        let (tx, rx) = oneshot::channel();
        if !self.tx.send(Command::Query { request, reply: tx }).is_ok() {
            return Err(Arc::new(anyhow::anyhow!("Not ready.")));
        }
        rx.await
            .map_err(|_| Arc::new(anyhow::anyhow!("Not ready.")))?
    }
}

async fn do_connect(url: &str, delay: Option<Duration>) -> Result<(WsStream, Protocols)> {
    if let Some(delay) = delay {
        tokio::time::sleep(delay).await;
    }

    tracing::error!(url = %url, "Connection to websocket.");

    const PROTOCOLS: &str = "graphql-ws, graphql-transport-ws";
    let req = HttpRequest::builder()
        .uri(url)
        .header("Sec-WebSocket-Protocol", PROTOCOLS)
        .body(())
        .with_context(|| "Invalid url")?;
    let (mut stream, http_resp) = tokio_tungstenite::connect_async(req)
        .await
        .with_context(|| "Failed to connect to websocket endpoint")?;
    let protocol = http_resp
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|value| value.to_str().ok())
        .map(|value| match value {
            "graphql-ws" => Some(Protocols::SubscriptionsTransportWS),
            "graphql-transport-ws" => Some(Protocols::GraphQLWS),
            _ => None,
        })
        .flatten()
        .ok_or_else(|| anyhow::anyhow!("Unknown protocol: {}", url))?;

    tracing::error!(url = %url, "Send connection_init.");
    stream
        .send(Message::Text(
            serde_json::to_string(&ClientMessage::ConnectionInit { payload: None }).unwrap(),
        ))
        .await?;

    loop {
        let reply = stream
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("Connection closed by server."))?
            .with_context(|| "Connection error")?;

        if let Message::Text(text) = reply {
            let message =
                serde_json::from_str::<ServerMessage>(&text).with_context(|| "Invalid response")?;
            match message {
                ServerMessage::ConnectionAck => {
                    tracing::error!(url = %url, "Received connection_ack");
                    break;
                }
                ServerMessage::ConnectionError {
                    payload: ConnectionError { message },
                } => {
                    return Err(anyhow::anyhow!("{}", message));
                }
                _ => {}
            }
        }
    }

    Ok((stream, protocol))
}

fn spawn_connect(
    url: String,
    delay: Option<Duration>,
    tx: mpsc::UnboundedSender<Result<(WsStream, Protocols)>>,
) {
    tokio::spawn(async move {
        tx.send(do_connect(&url, delay).await).ok();
    });
}

async fn main_loop(mut rx: mpsc::UnboundedReceiver<Command>, url: String) {
    let mut stream = Either::Right(pending());
    let mut sink = None;
    let mut protocol = Protocols::SubscriptionsTransportWS;
    let (tx_connect, mut rx_connect) = mpsc::unbounded_channel();
    let mut pending_requests: HashMap<String, oneshot::Sender<Result<Response, Arc<Error>>>> =
        HashMap::new();
    let mut req_id = 0usize;

    spawn_connect(url.clone(), None, tx_connect.clone());

    loop {
        tokio::select! {
            connect_resp = rx_connect.recv() => {
                match connect_resp {
                    Some(Ok(resp)) => {
                        let s = resp.0.split();
                        stream = Either::Left(s.1);
                        sink = Some(s.0);
                        protocol = resp.1;
                    }
                    Some(Err(err)) => {
                        tracing::error!(url = %url, error = %err, "Failed to connect to websocket");
                        spawn_connect(url.clone(), Some(Duration::from_secs(RECONNECT_DELAY_SECONDS)), tx_connect.clone());
                    }
                    None => {}
                }
            }
            message = stream.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(message) = serde_json::from_str::<ServerMessage>(&text) {
                            match message {
                                ServerMessage::Data { id, payload } => {
                                    if let Some(sender) = pending_requests.remove(id) {
                                        sender.send(Ok(payload)).ok();
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        tracing::error!(url = %url, error = %err, "Connection error.");
                        let err = Arc::new(anyhow::anyhow!("{}", err));
                        pending_requests.drain().for_each(|(_, sender)| {
                            sender.send(Err(err.clone())).ok();
                        });
                        sink = None;
                        stream = Either::Right(pending());
                        spawn_connect(url.clone(), Some(Duration::from_secs(RECONNECT_DELAY_SECONDS)), tx_connect.clone());
                    }
                    None => {
                        tracing::error!(url = %url, "Connection closed by server.");
                        let err = Arc::new(anyhow::anyhow!("Connection closed by server."));
                        pending_requests.drain().for_each(|(_, sender)| {
                            sender.send(Err(err.clone())).ok();
                        });
                        sink = None;
                        stream = Either::Right(pending());
                        spawn_connect(url.clone(), Some(Duration::from_secs(RECONNECT_DELAY_SECONDS)), tx_connect.clone());
                    }
                }
            }
            command = rx.recv() => {
                match command {
                    Some(Command::IsReady { reply }) => {
                        reply.send(sink.is_some()).ok();
                    }
                    Some(Command::Query { request, reply }) => {
                        if let Some(sink) = &mut sink {
                            req_id += 1;
                            let id = format!("{}", req_id);
                            pending_requests.insert(id.clone(), reply);
                            let msg = match protocol {
                                Protocols::SubscriptionsTransportWS => {
                                    Message::text(
                                        serde_json::to_string(&ClientMessage::Start { id: &id, payload: request }
                                    ).unwrap())
                                }
                                Protocols::GraphQLWS => {
                                    Message::text(
                                        serde_json::to_string(&ClientMessage::Subscribe { id: &id, payload: request }
                                    ).unwrap())
                                }
                            };
                            sink.send(msg).await.ok();
                        } else {
                            reply.send(Err(Arc::new(anyhow::anyhow!("Not ready.")))).ok();
                        }
                    }
                    None => return,
                }
            }
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum Protocols {
    /// [subscriptions-transport-ws protocol](https://github.com/apollographql/subscriptions-transport-ws/blob/master/PROTOCOL.md).
    SubscriptionsTransportWS,
    /// [graphql-ws protocol](https://github.com/enisdenjo/graphql-ws/blob/master/PROTOCOL.md).
    GraphQLWS,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
enum ClientMessage<'a> {
    ConnectionInit { payload: Option<serde_json::Value> },
    Start { id: &'a str, payload: Request },
    Subscribe { id: &'a str, payload: Request },
    Stop { id: &'a str },
    Complete { id: &'a str },
    ConnectionTerminate,
}

#[derive(Deserialize)]
struct ConnectionError<'a> {
    message: &'a str,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
enum ServerMessage<'a> {
    ConnectionError {
        payload: ConnectionError<'a>,
    },
    ConnectionAck,
    #[serde(alias = "next")]
    Data {
        id: &'a str,
        payload: Response,
    },
    Complete {
        id: &'a str,
    },
}
