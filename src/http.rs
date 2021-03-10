#![allow(dead_code)]

use std::borrow::Borrow;
use std::collections::HashMap;
use std::convert::Infallible;
use std::hash::Hash;
use std::pin::Pin;
use std::str::FromStr;
use std::task::{Context, Poll};

use anyhow::Error;
use futures_util::stream::Stream;
use futures_util::task::AtomicWaker;
use futures_util::{SinkExt, StreamExt};
use graphgate_core::{Request, Response, ServerError};
use serde::{Deserialize, Serialize};
use warp::http::Response as HttpResponse;
use warp::ws::{Message, Ws};
use warp::{Filter, Rejection, Reply};

use crate::SharedCoordinator;

struct GroupedStream<K, S> {
    streams: HashMap<K, S>,
    waker: AtomicWaker,
}

impl<K, S> Default for GroupedStream<K, S> {
    fn default() -> Self {
        Self {
            streams: Default::default(),
            waker: Default::default(),
        }
    }
}

impl<K: Eq + Hash + Clone, S> GroupedStream<K, S> {
    #[inline]
    fn insert(&mut self, key: K, stream: S) {
        self.waker.wake();
        self.streams.insert(key, stream);
    }

    #[inline]
    fn remove<Q: ?Sized>(&mut self, key: &Q)
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.streams.remove(key);
    }
}

enum StreamEvent<K, T> {
    Data(K, T),
    Complete(K),
}

impl<K, T, S> Stream for GroupedStream<K, S>
where
    K: Eq + Hash + Clone + Unpin,
    S: Stream<Item = T> + Unpin,
{
    type Item = StreamEvent<K, T>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.waker.register(cx.waker());

        for (key, stream) in self.streams.iter_mut() {
            match stream.poll_next_unpin(cx) {
                Poll::Ready(Some(value)) => {
                    return Poll::Ready(Some(StreamEvent::Data(key.clone(), value)))
                }
                Poll::Ready(None) => {
                    let key = key.clone();
                    self.streams.remove(&key);
                    return Poll::Ready(Some(StreamEvent::Complete(key)));
                }
                Poll::Pending => {}
            }
        }

        Poll::Pending
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage<'a> {
    ConnectionInit {
        payload: Option<serde_json::Value>,
    },
    #[serde(alias = "subscribe")]
    Start {
        id: String,
        payload: Request,
    },
    #[serde(alias = "complete")]
    Stop {
        id: &'a str,
    },
    ConnectionTerminate,
}

/// Specification of which GraphQL Over WebSockets protocol is being utilized
#[derive(Copy, Clone)]
pub enum Protocols {
    /// [subscriptions-transport-ws protocol](https://github.com/apollographql/subscriptions-transport-ws/blob/master/PROTOCOL.md).
    SubscriptionsTransportWS,
    /// [graphql-ws protocol](https://github.com/enisdenjo/graphql-ws/blob/master/PROTOCOL.md).
    GraphQLWS,
}

impl Protocols {
    /// Returns the `Sec-WebSocket-Protocol` header value for the protocol
    pub fn sec_websocket_protocol(&self) -> &str {
        match self {
            Protocols::SubscriptionsTransportWS => "graphql-ws",
            Protocols::GraphQLWS => "graphql-transport-ws",
        }
    }

    #[inline]
    fn next_message<'s>(&self, id: &'s str, payload: Response) -> ServerMessage<'s> {
        match self {
            Protocols::SubscriptionsTransportWS => ServerMessage::Data { id, payload },
            Protocols::GraphQLWS => ServerMessage::Next { id, payload },
        }
    }
}

impl std::str::FromStr for Protocols {
    type Err = Error;

    fn from_str(protocol: &str) -> Result<Self, Self::Err> {
        if protocol.eq_ignore_ascii_case("graphql-ws") {
            Ok(Protocols::SubscriptionsTransportWS)
        } else if protocol.eq_ignore_ascii_case("graphql-transport-ws") {
            Ok(Protocols::GraphQLWS)
        } else {
            Err(anyhow::anyhow!(
                "Unsupported Sec-WebSocket-Protocol: {}",
                protocol
            ))
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage<'a> {
    ConnectionError {
        payload: ServerError,
    },
    ConnectionAck,
    /// subscriptions-transport-ws protocol next payload
    Data {
        id: &'a str,
        payload: Response,
    },
    /// graphql-ws protocol next payload
    Next {
        id: &'a str,
        payload: Response,
    },
    // Not used by this library, as it's not necessary to send
    // Error {
    //     id: &'a str,
    //     payload: serde_json::Value,
    // },
    Complete {
        id: &'a str,
    },
    // Not used by this library
    // #[serde(rename = "ka")]
    // KeepAlive
}

pub fn graphql(
    shared_coordinator: SharedCoordinator,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    let graphql = warp::post().and(warp::body::json()).and_then({
        let shared_coordinator = shared_coordinator.clone();
        move |request: Request| {
            let shared_coordinator = shared_coordinator.clone();
            async move { Ok::<_, Infallible>(shared_coordinator.query(request).await) }
        }
    });

    let graphql_ws = warp::ws()
        .and(warp::header::optional::<String>("sec-websocket-protocol"))
        .map({
            move |ws: Ws, protocols: Option<String>| {
                let protocol = protocols
                    .and_then(|protocols| {
                        protocols
                            .split(',')
                            .find_map(|p| Protocols::from_str(p.trim()).ok())
                    })
                    .unwrap_or(Protocols::SubscriptionsTransportWS);
                handle_websocket(ws, shared_coordinator.clone(), protocol)
            }
        });

    let playground = warp::get().map(|| {
        HttpResponse::builder()
            .header("content-type", "text/html")
            .body(include_str!("playground.html"))
    });

    graphql.or(graphql_ws).or(playground)
}

fn handle_websocket(
    ws: Ws,
    shared_coordinator: SharedCoordinator,
    protocol: Protocols,
) -> impl Reply {
    let reply = ws.on_upgrade(move |websocket| {
        let (mut sink, mut stream) = websocket.split();
        let shared_coordinator = shared_coordinator.clone();
        let mut streams = GroupedStream::default();

        async move {
            loop {
                tokio::select! {
                    message = stream.next() => match message {
                        Some(Ok(message)) if message.is_text() => {
                            match serde_json::from_slice::<ClientMessage>(message.as_bytes()) {
                                Ok(ClientMessage::ConnectionInit { payload: _ }) => {
                                    let data = match serde_json::to_string(&ServerMessage::ConnectionAck) {
                                        Ok(data) => data,
                                        Err(_) => break,
                                    };
                                    if sink.send(Message::text(data)).await.is_err() {
                                        break;
                                    }
                                }
                                Ok(ClientMessage::Start { id, payload }) => {
                                    streams.insert(id, shared_coordinator.subscribe(payload).await);
                                }
                                Ok(ClientMessage::Stop { id }) => {
                                    streams.remove(id);
                                }
                                Ok(ClientMessage::ConnectionTerminate) => break,
                                Err(_) => break,
                            }
                        }
                        _ => break,
                    },
                    event = streams.next() => match event {
                        Some(StreamEvent::Data(id, response)) => {
                            let data = match serde_json::to_string(&protocol.next_message(&id, response)) {
                                Ok(data) => data,
                                Err(_) => break,
                            };
                            if sink.send(Message::text(data)).await.is_err() {
                                break;
                            }
                        }
                        Some(StreamEvent::Complete(id)) => {
                            let data = match serde_json::to_string(&ServerMessage::Complete {
                                id: &id,
                            }) {
                                Ok(data) => data,
                                Err(_) => break,
                            };
                            if sink.send(Message::text(data)).await.is_err() {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    warp::reply::with_header(
        reply,
        "Sec-WebSocket-Protocol",
        protocol.sec_websocket_protocol(),
    )
}
