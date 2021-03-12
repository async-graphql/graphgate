use std::convert::{Infallible, TryInto};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use graphgate_core::{Request, WebSocketProtocols};
use warp::http::header::HeaderName;
use warp::http::{HeaderMap, Response as HttpResponse};
use warp::ws::Ws;
use warp::{Filter, Rejection, Reply};

use crate::config::Config;
use crate::shared_route_table::SharedRouteTable;

fn forward_headers(
    config: &Config,
    header_map: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> HeaderMap {
    let mut new_header_map = HeaderMap::new();
    for name in &config.forward_headers {
        for value in header_map.get_all(name) {
            if let Ok(name) = HeaderName::from_str(name) {
                new_header_map.append(name, value.clone());
            }
        }
    }
    if let Some(remote_addr) = remote_addr {
        if let Ok(remote_addr) = remote_addr.to_string().try_into() {
            new_header_map.append(warp::http::header::FORWARDED, remote_addr);
        }
    }
    new_header_map
}

pub fn graphql(
    shared_route_table: SharedRouteTable,
    config: Arc<Config>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    let graphql = warp::post()
        .and(warp::body::json())
        .and(warp::header::headers_cloned())
        .and(warp::addr::remote())
        .and_then({
            let route_table = shared_route_table.clone();
            let config = config.clone();
            move |request: Request, header_map: HeaderMap, remote_addr: Option<SocketAddr>| {
                let route_table = route_table.clone();
                let config = config.clone();
                async move {
                    Ok::<_, Infallible>(
                        route_table
                            .query(request, forward_headers(&config, &header_map, remote_addr))
                            .await,
                    )
                }
            }
        });

    let graphql_ws = warp::ws()
        .and(warp::get())
        .and(warp::header::optional::<String>("sec-websocket-protocol"))
        .and(warp::header::headers_cloned())
        .and(warp::addr::remote())
        .map({
            move |ws: Ws, protocols: Option<String>, header_map, remote_addr: Option<SocketAddr>| {
                let shared_route_table = shared_route_table.clone();
                let config = config.clone();
                let protocol = protocols
                    .and_then(|protocols| {
                        protocols
                            .split(',')
                            .find_map(|p| WebSocketProtocols::from_str(p.trim()).ok())
                    })
                    .unwrap_or(WebSocketProtocols::SubscriptionsTransportWS);
                let header_map = forward_headers(&config, &header_map, remote_addr);

                let reply = ws.on_upgrade(move |websocket| async move {
                    if let Some((composed_schema, route_table)) = shared_route_table.get().await {
                        graphgate_core::websocket_server(
                            composed_schema,
                            route_table,
                            websocket,
                            protocol,
                            header_map,
                        )
                        .await;
                    }
                });

                warp::reply::with_header(
                    reply,
                    "Sec-WebSocket-Protocol",
                    protocol.sec_websocket_protocol(),
                )
            }
        });

    let playground = warp::get().map(|| {
        HttpResponse::builder()
            .header("content-type", "text/html")
            .body(include_str!("playground.html"))
    });

    graphql.or(graphql_ws).or(playground)
}
