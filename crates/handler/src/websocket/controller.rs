use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::Arc,
};

use anyhow::Result;
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use graphgate_planner::{Request, Response};
use http::{HeaderMap, Request as HttpRequest};
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot},
    time::Duration,
};
use tokio_tungstenite::{
    tungstenite::{protocol::CloseFrame, Message, Result as WsResult},
    MaybeTlsStream, WebSocketStream,
};

use super::{
    grouped_stream::{GroupedStream, StreamEvent},
    protocol::{ClientMessage, Protocols, ServerMessage},
};
use crate::ServiceRouteTable;

const CONNECT_TIMEOUT_SECONDS: u64 = 5;

#[derive(Debug)]
struct SubscribeCommand {
    service: String,
    id: String,
    payload: Request,
    tx: mpsc::UnboundedSender<Response>,
    reply: oneshot::Sender<Result<()>>,
}

struct StopCommand {
    id: String,
}

enum Command {
    Subscribe(SubscribeCommand),
    Stop(StopCommand),
}

#[derive(Clone)]
pub struct WebSocketController {
    tx_command: mpsc::UnboundedSender<Command>,
}

impl WebSocketController {
    pub fn new(
        route_table: Arc<ServiceRouteTable>,
        header_map: &HeaderMap,
        init_payload: Option<serde_json::Value>,
    ) -> Self {
        let (tx_command, rx_command) = mpsc::unbounded_channel();
        let ctx = WebSocketContext {
            route_table,
            header_map: header_map.clone(),
            init_payload,
            upstream: GroupedStream::default(),
            upstream_info: Default::default(),
            rx_command,
            subscribes: Default::default(),
        };

        tokio::spawn(ctx.main());
        Self { tx_command }
    }

    pub async fn subscribe(
        &self,
        id: impl Into<String>,
        service: impl Into<String>,
        request: Request,
        tx: mpsc::UnboundedSender<Response>,
    ) -> Result<()> {
        let (tx_reply, rx_reply) = oneshot::channel();
        if self
            .tx_command
            .send(Command::Subscribe(SubscribeCommand {
                service: service.into(),
                id: id.into(),
                payload: request,
                tx,
                reply: tx_reply,
            }))
            .is_err()
        {
            anyhow::bail!("Connection closed.");
        }
        rx_reply
            .await
            .map_err(|_| anyhow::anyhow!("Connection closed."))?
    }

    pub async fn stop(&self, id: impl Into<String>) {
        self.tx_command
            .send(Command::Stop(StopCommand { id: id.into() }))
            .ok();
    }
}

struct UpstreamInfo {
    protocol: Protocols,
    sink: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    subscribe_count: usize,
}

struct SubscribeInfo {
    services: HashSet<String>,
    tx: mpsc::UnboundedSender<Response>,
}

struct WebSocketContext {
    route_table: Arc<ServiceRouteTable>,
    header_map: HeaderMap,
    init_payload: Option<serde_json::Value>,
    upstream: GroupedStream<String, SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>,
    upstream_info: HashMap<String, UpstreamInfo>,
    rx_command: mpsc::UnboundedReceiver<Command>,
    subscribes: HashMap<String, SubscribeInfo>,
}

impl WebSocketContext {
    pub async fn main(mut self) {
        loop {
            tokio::select! {
                command = self.rx_command.recv() => match command {
                    Some(command) => self.handle_command(command).await,
                    None => return,
                },
                event = self.upstream.next() => match event {
                    Some(event) => if !self.handle_event(event).await {
                        return;
                    },
                    None => return,
                }
            }
        }
    }

    async fn handle_command(&mut self, command: Command) {
        match command {
            Command::Subscribe(command) => self.handle_command_subscribe(command).await,
            Command::Stop(command) => self.handle_command_stop(command).await,
        }
    }

    async fn ensure_upstream(
        &mut self,
        service: &str,
    ) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Protocols)> {
        const PROTOCOLS: &str = "graphql-ws, graphql-transport-ws";
        let route = self.route_table.get(service).ok_or_else(|| {
            anyhow::anyhow!("Service '{}' is not defined in the routing table.", service)
        })?;
        let scheme = match route.tls {
            true => "wss",
            false => "ws",
        };

        let url = match &route.websocket_path {
            Some(path) => format!("{}://{}{}", scheme, route.addr, path),
            None => format!("{}://{}", scheme, route.addr),
        };

        tracing::debug!(url = %url, service = service, "Connect to upstream websocket");
        let mut http_request = HttpRequest::builder()
            .uri(&url)
            .header("Sec-WebSocket-Protocol", PROTOCOLS)
            .body(())
            .unwrap();
        http_request.headers_mut().extend(self.header_map.clone());
        let (mut stream, http_response) = tokio_tungstenite::connect_async(http_request).await?;
        let protocol = http_response
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| Protocols::from_str(value).ok())
            .ok_or_else(|| anyhow::anyhow!("Unknown protocol: {}", url))?;

        stream
            .send(Message::Text(
                serde_json::to_string(&ClientMessage::ConnectionInit {
                    payload: self.init_payload.clone(),
                })
                .unwrap(),
            ))
            .await?;

        let timeout = tokio::time::sleep(Duration::from_secs(CONNECT_TIMEOUT_SECONDS));
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                _ = &mut timeout => return Err(anyhow::anyhow!("Connect timeout.")),
                message = stream.next() => match message {
                    Some(Ok(Message::Text(text))) => {
                        let message = serde_json::from_str::<ServerMessage>(&text).map_err(|_| anyhow::anyhow!("Invalid response"))?;
                        match message {
                            ServerMessage::ConnectionAck => break,
                            ServerMessage::ConnectionError { payload } => return Err(anyhow::anyhow!("Connection error. {}", payload.message)),
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        stream.send(Message::Pong(data)).await?;
                    }
                    Some(Ok(Message::Close(Some(CloseFrame{ code, reason })))) => return Err(anyhow::anyhow!("Connection closed by server, code={} reason={}", code, reason)),
                    Some(Err(err)) => return Err(anyhow::anyhow!("Connection error. {}", err)),
                    Some(Ok(Message::Close(None))) | None => return Err(anyhow::anyhow!("Connection closed by server.")),
                    Some(Ok(_)) => {}
                }
            }
        }

        tracing::debug!(url = %url, service = service, protocol = ?protocol, "upstream websocket connected.");
        Ok((stream, protocol))
    }

    async fn handle_command_subscribe(&mut self, command: SubscribeCommand) {
        if !self.upstream.contains_key(&command.service) {
            let (stream, protocol) = match self.ensure_upstream(&command.service).await {
                Ok(stream) => stream,
                Err(err) => {
                    command.reply.send(Err(err)).ok();
                    return;
                }
            };
            let (sink, stream) = stream.split();
            self.upstream.insert(command.service.clone(), stream);
            self.upstream_info.insert(
                command.service.clone(),
                UpstreamInfo {
                    protocol,
                    sink,
                    subscribe_count: 0,
                },
            );
        }

        if let Some(info) = self.upstream_info.get_mut(&command.service) {
            info.subscribe_count += 1;

            match self.subscribes.get_mut(&command.id) {
                Some(subscribe_info) => {
                    assert!(!subscribe_info.services.contains(&command.service));
                    subscribe_info.services.insert(command.service.clone());
                }
                None => {
                    self.subscribes.insert(
                        command.id.clone(),
                        SubscribeInfo {
                            services: std::iter::once(command.service.clone()).collect(),
                            tx: command.tx,
                        },
                    );
                }
            }

            info.sink
                .send(Message::text(
                    serde_json::to_string(
                        &info
                            .protocol
                            .subscribe_message(&command.id, command.payload),
                    )
                    .unwrap(),
                ))
                .await
                .ok();

            command.reply.send(Ok(())).ok();
        }
    }

    fn finish_subscribe(&mut self, id: &str) {
        if let Some(subscribe_info) = self.subscribes.remove(id) {
            for service in subscribe_info.services {
                if let Some(upstream_info) = self.upstream_info.get_mut(&service) {
                    upstream_info.subscribe_count -= 1;
                    if upstream_info.subscribe_count == 0 {
                        self.upstream_info.remove(&service);
                        self.upstream.remove(&service);
                        tracing::debug!(service = %service, "Close upstream websocket");
                    }
                }
            }
        }
    }

    async fn handle_command_stop(&mut self, command: StopCommand) {
        self.finish_subscribe(&command.id);
    }

    async fn handle_event(&mut self, event: StreamEvent<String, WsResult<Message>>) -> bool {
        match event {
            StreamEvent::Data(_, Ok(Message::Text(text))) => {
                let message = match serde_json::from_str::<ServerMessage>(&text) {
                    Ok(message) => message,
                    Err(_) => return false,
                };
                match message {
                    ServerMessage::Data { id, payload } | ServerMessage::Next { id, payload } => {
                        if let Some(info) = self.subscribes.get_mut(id) {
                            if info.tx.send(payload).is_err() {
                                self.finish_subscribe(id);
                            }
                        }
                    }
                    ServerMessage::Complete { id } => {
                        self.finish_subscribe(id);
                    }
                    _ => {}
                }
                true
            }
            StreamEvent::Data(service, Ok(Message::Ping(data))) => {
                if let Some(info) = self.upstream_info.get_mut(&service) {
                    info.sink.send(Message::Pong(data)).await.ok();
                }
                true
            }
            StreamEvent::Data(_, Ok(_)) => true,
            StreamEvent::Data(_, Err(_)) | StreamEvent::Complete(_) => false,
        }
    }
}
