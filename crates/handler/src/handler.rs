use std::convert::{Infallible, TryInto};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use graphgate_planner::Request;
use http::header::HeaderName;
use http::HeaderMap;
use warp::http::Response as HttpResponse;
use warp::ws::Ws;
use warp::{Filter, Rejection, Reply};

use crate::{websocket, SharedRouteTable};

#[derive(Clone)]
pub struct HandlerConfig {
    pub shared_route_table: SharedRouteTable,
    pub forward_headers: Arc<Vec<String>>,
}

fn do_forward_headers<T: AsRef<str>>(
    forward_headers: &[T],
    header_map: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> HeaderMap {
    let mut new_header_map = HeaderMap::new();
    for name in forward_headers {
        for value in header_map.get_all(name.as_ref()) {
            if let Ok(name) = HeaderName::from_str(name.as_ref()) {
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

pub fn graphql_request(
    config: HandlerConfig,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    warp::post()
        .and(warp::body::json())
        .and(warp::header::headers_cloned())
        .and(warp::addr::remote())
        .and_then({
            move |request: Request, header_map: HeaderMap, remote_addr: Option<SocketAddr>| {
                let config = config.clone();
                async move {
                    let resp = config
                        .shared_route_table
                        .query(
                            request,
                            do_forward_headers(&config.forward_headers, &header_map, remote_addr),
                        )
                        .await;
                    Ok::<_, Infallible>(resp)
                }
            }
        })
}

pub fn graphql_websocket(
    config: HandlerConfig,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    warp::ws()
        .and(warp::get())
        .and(warp::header::exact_ignore_case("upgrade", "websocket"))
        .and(warp::header::optional::<String>("sec-websocket-protocol"))
        .and(warp::header::headers_cloned())
        .and(warp::addr::remote())
        .map({
            move |ws: Ws, protocols: Option<String>, header_map, remote_addr: Option<SocketAddr>| {
                let config = config.clone();
                let protocol = protocols
                    .and_then(|protocols| {
                        protocols
                            .split(',')
                            .find_map(|p| websocket::Protocols::from_str(p.trim()).ok())
                    })
                    .unwrap_or(websocket::Protocols::SubscriptionsTransportWS);
                let header_map =
                    do_forward_headers(&config.forward_headers, &header_map, remote_addr);

                let reply = ws.on_upgrade(move |websocket| async move {
                    if let Some((composed_schema, route_table)) =
                        config.shared_route_table.get().await
                    {
                        websocket::server(
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
        })
}

pub fn graphql_playground() -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    warp::get().map(|| {
        HttpResponse::builder()
            .header("content-type", "text/html")
            .body(include_str!("playground.html"))
    })
}
