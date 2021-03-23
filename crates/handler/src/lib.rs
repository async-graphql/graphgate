#![forbid(unsafe_code)]

mod constants;
mod executor;
mod fetcher;
mod introspection;
mod service_route;
mod shared_route_table;
mod websocket;

pub mod handler;

pub use service_route::{ServiceRoute, ServiceRouteTable};
pub use shared_route_table::SharedRouteTable;
