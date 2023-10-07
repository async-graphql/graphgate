use std::{
    convert::{Infallible, TryInto},
    net::SocketAddr,
    str::FromStr,
    sync::Arc,
    time::Instant,
};

use graphgate_planner::Request;
use http::{header::HeaderName, HeaderMap};
use opentelemetry::{
    global,
    trace::{FutureExt, TraceContextExt, Tracer},
    Context,
};
use warp::{http::Response as HttpResponse, ws::Ws, Filter, Rejection, Reply};

use crate::{constants::*, metrics::METRICS, websocket, SharedRouteTable};

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
                    let tracer = global::tracer("graphql");

                    let query = Context::current_with_span(
                        tracer
                            .span_builder("query")
                            .with_attributes(vec![
                                KEY_QUERY.string(request.query.clone()),
                                KEY_VARIABLES
                                    .string(serde_json::to_string(&request.variables).unwrap()),
                            ])
                            .start(&tracer),
                    );

                    let start_time = Instant::now();
                    let resp = config
                        .shared_route_table
                        .query(
                            request,
                            do_forward_headers(&config.forward_headers, &header_map, remote_addr),
                        )
                        .with_context(query)
                        .await;

                    METRICS
                        .query_histogram
                        .record((Instant::now() - start_time).as_secs_f64(), &[]);
                    METRICS.query_counter.add(1, &[]);

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
