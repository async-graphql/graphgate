use graphgate_core::Request;
use std::convert::Infallible;
use warp::http::Response as HttpResponse;
use warp::{Filter, Rejection, Reply};

use crate::SharedCoordinator;

pub fn graphql_filter(
    shared_coordinator: SharedCoordinator,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    let graphql = warp::post().and(warp::body::json()).and_then({
        let shared_coordinator = shared_coordinator.clone();
        move |request: Request| {
            let shared_coordinator = shared_coordinator.clone();
            async move { Ok::<_, Infallible>(shared_coordinator.query(request).await) }
        }
    });

    let playground = warp::get().map(|| {
        HttpResponse::builder()
            .header("content-type", "text/html")
            .body(include_str!("playground.html"))
    });

    graphql.or(playground)
}
