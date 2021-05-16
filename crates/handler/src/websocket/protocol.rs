use anyhow::Error;
use graphgate_planner::{Request, Response};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Protocols {
    /// [subscriptions-transport-ws protocol](https://github.com/apollographql/subscriptions-transport-ws/blob/master/PROTOCOL.md).
    SubscriptionsTransportWS,
    /// [graphql-ws protocol](https://github.com/enisdenjo/graphql-ws/blob/master/PROTOCOL.md).
    GraphQLWS,
}

impl std::str::FromStr for Protocols {
    type Err = Error;

    fn from_str(protocol: &str) -> Result<Self, Self::Err> {
        if protocol.eq_ignore_ascii_case("graphql-ws") {
            Ok(Protocols::SubscriptionsTransportWS)
        } else if protocol.eq_ignore_ascii_case("graphql-transport-ws") {
            Ok(Protocols::GraphQLWS)
        } else {
            anyhow::bail!("Unsupported Sec-WebSocket-Protocol: {}", protocol)
        }
    }
}

impl Protocols {
    pub fn sec_websocket_protocol(&self) -> &str {
        match self {
            Protocols::SubscriptionsTransportWS => "graphql-ws",
            Protocols::GraphQLWS => "graphql-transport-ws",
        }
    }

    #[inline]
    pub fn subscribe_message<'a>(&self, id: &'a str, request: Request) -> ClientMessage<'a> {
        match self {
            Protocols::SubscriptionsTransportWS => ClientMessage::Start {
                id,
                payload: request,
            },
            Protocols::GraphQLWS => ClientMessage::Subscribe {
                id,
                payload: request,
            },
        }
    }

    #[inline]
    pub fn next_message<'a>(&self, id: &'a str, payload: Response) -> ServerMessage<'a> {
        match self {
            Protocols::SubscriptionsTransportWS => ServerMessage::Data { id, payload },
            Protocols::GraphQLWS => ServerMessage::Next { id, payload },
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ClientMessage<'a> {
    ConnectionInit { payload: Option<serde_json::Value> },
    Start { id: &'a str, payload: Request },
    Subscribe { id: &'a str, payload: Request },
    Stop { id: &'a str },
    Complete { id: &'a str },
    ConnectionTerminate,
}

#[derive(Deserialize, Serialize)]
pub struct ConnectionError<'a> {
    pub message: &'a str,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ServerMessage<'a> {
    ConnectionError { payload: ConnectionError<'a> },
    ConnectionAck,
    Data { id: &'a str, payload: Response },
    Next { id: &'a str, payload: Response },
    Complete { id: &'a str },
}
