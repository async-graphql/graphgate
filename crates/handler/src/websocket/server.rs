use std::sync::Arc;

use futures_util::sink::Sink;
use futures_util::stream::Stream;
use futures_util::{SinkExt, StreamExt};
use graphgate_planner::{PlanBuilder, Response, ServerError};
use graphgate_schema::ComposedSchema;
use value::ConstValue;
use warp::http::HeaderMap;
use warp::ws::Message;
use warp::Error;

use super::controller::WebSocketController;
use super::grouped_stream::{GroupedStream, StreamEvent};
use super::protocol::{ClientMessage, ConnectionError, Protocols, ServerMessage};
use crate::executor::Executor;
use crate::ServiceRouteTable;

pub async fn server(
    schema: Arc<ComposedSchema>,
    route_table: Arc<ServiceRouteTable>,
    stream: impl Stream<Item = Result<Message, Error>> + Sink<Message>,
    protocol: Protocols,
    header_map: HeaderMap,
) {
    let (mut sink, mut stream) = stream.split();
    let mut streams = GroupedStream::default();
    let mut controller = None;
    let header_map = Arc::new(header_map);

    loop {
        tokio::select! {
            message = stream.next() => match message {
                Some(Ok(message)) if message.is_text() => {
                    let text = message.into_bytes();
                    let client_msg = match serde_json::from_slice::<ClientMessage>(&text) {
                        Ok(client_msg) => client_msg,
                        Err(_) => return,
                    };

                    match client_msg {
                        ClientMessage::ConnectionInit { payload } if controller.is_none() => {
                            controller = Some(WebSocketController::new(route_table.clone(), &header_map, payload));
                            sink.send(Message::text(serde_json::to_string(&ServerMessage::ConnectionAck).unwrap())).await.ok();
                        }
                        ClientMessage::ConnectionInit { .. } => {
                            match protocol {
                                Protocols::SubscriptionsTransportWS => {
                                    let err_msg = Message::text(
                                        serde_json::to_string(&ServerMessage::ConnectionError {
                                            payload: ConnectionError {
                                                message: "Too many initialisation requests.",
                                            },
                                        }).unwrap());
                                    sink.send(err_msg).await.ok();
                                    return;
                                }
                                Protocols::GraphQLWS => {
                                    sink.send(Message::close_with(4429u16, "Too many initialisation requests.")).await.ok();
                                    return;
                                }
                            }
                        }
                        ClientMessage::Start { id, payload } | ClientMessage::Subscribe { id, payload } => {
                            let controller = controller.get_or_insert_with(|| WebSocketController::new(route_table.clone(), &header_map, None)).clone();
                            let document = match parser::parse_query(&payload.query) {
                                Ok(document) => document,
                                Err(err) => {
                                    let resp = Response {
                                        data: ConstValue::Null,
                                        errors: vec![ServerError::new(err.to_string())],
                                        extensions: Default::default(),
                                        headers: Default::default()
                                    };
                                    let data = ServerMessage::Data { id, payload: resp };
                                    sink.send(Message::text(serde_json::to_string(&data).unwrap())).await.ok();

                                    let complete = ServerMessage::Complete { id };
                                    sink.send(Message::text(serde_json::to_string(&complete).unwrap())).await.ok();
                                    continue;
                                }
                            };

                            let id = Arc::new(id.to_string());
                            let schema = schema.clone();
                            let stream = {
                                let id = id.clone();
                                async_stream::stream! {
                                    let builder = PlanBuilder::new(&schema, document).variables(payload.variables);
                                    let node = match builder.plan() {
                                        Ok(node) => node,
                                        Err(resp) => {
                                            yield resp;
                                            return;
                                        }
                                    };
                                    let executor = Executor::new(&schema);
                                    let mut stream = executor.execute_stream(controller.clone(), &id, &node).await;
                                    while let Some(item) = stream.next().await {
                                        yield item;
                                    }
                                }
                            };
                            streams.insert(id, Box::pin(stream));
                        }
                        ClientMessage::Stop { id } => {
                            let controller = controller.get_or_insert_with(|| WebSocketController::new(route_table.clone(), &header_map, None)).clone();
                            controller.stop(id).await;
                        }
                        _ => {}
                    }
                }
                Some(Ok(message)) if message.is_close() => return,
                Some(Err(_)) | None => return,
                _ => {}
            },
            item = streams.next() => if let Some(event) = item {
                match event {
                    StreamEvent::Data(id, resp) => {
                        let data = protocol.next_message(&id, resp);
                        if sink.send(Message::text(serde_json::to_string(&data).unwrap())).await.is_err() {
                            return;
                        }
                    }
                    StreamEvent::Complete(id) => {
                        let complete = ServerMessage::Complete { id: &id };
                        if sink.send(Message::text(serde_json::to_string(&complete).unwrap())).await.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    }
}
