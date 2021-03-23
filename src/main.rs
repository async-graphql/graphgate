#![forbid(unsafe_code)]

mod config;
mod k8s;
mod options;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures_util::FutureExt;
use graphgate_handler::handler::HandlerConfig;
use graphgate_handler::{handler, SharedRouteTable};
use opentelemetry::global;
use opentelemetry::trace::NoopTracerProvider;
use structopt::StructOpt;
use tokio::signal;
use tokio::time::Duration;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};
use warp::Filter;

use config::Config;
use options::Options;

fn init_tracing() {
    tracing_subscriber::registry()
        .with(fmt::layer().compact().with_target(false))
        .with(
            EnvFilter::try_from_default_env()
                .or_else(|_| EnvFilter::try_new("info"))
                .unwrap(),
        )
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let options: Options = Options::from_args();
    init_tracing();

    let config = toml::from_str::<Config>(
        &std::fs::read_to_string(&options.config)
            .with_context(|| format!("Failed to load config file '{}'.", options.config))?,
    )
    .with_context(|| format!("Failed to parse config file '{}'.", options.config))?;

    let _uninstall = match &config.jaeger {
        Some(config) => {
            tracing::info!(
                agent_endpoint = %config.agent_endpoint,
                service_name = %config.service_name,
                "Initialize Jaeger"
            );
            let provider = opentelemetry_jaeger::new_pipeline()
                .with_agent_endpoint(&config.agent_endpoint)
                .with_service_name(&config.service_name)
                .build_batch(opentelemetry::runtime::Tokio)
                .context("Failed to initialize jaeger.")?;
            global::set_tracer_provider(provider)
        }
        None => {
            let provider = NoopTracerProvider::new();
            global::set_tracer_provider(provider)
        }
    };

    let shared_route_table = SharedRouteTable::default();
    if !config.services.is_empty() {
        tracing::info!("Routing table in the configuration file.");
        shared_route_table.set_route_table(config.create_route_table());
    } else {
        tracing::info!("Routing table within the current namespace in Kubernetes cluster.");
        tokio::spawn({
            let shared_route_table = shared_route_table.clone();
            async move {
                let mut prev_route_table = None;
                loop {
                    match k8s::find_graphql_services().await {
                        Ok(route_table) => {
                            if Some(&route_table) != prev_route_table.as_ref() {
                                tracing::info!(route_table = ?route_table, "Route table updated.");
                                shared_route_table.set_route_table(route_table.clone());
                                prev_route_table = Some(route_table);
                            }
                        }
                        Err(err) => {
                            tracing::error!(error = %err, "Failed to find graphql services.");
                        }
                    }

                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            }
        });
    }

    let handler_config = HandlerConfig {
        shared_route_table,
        forward_headers: Arc::new(config.forward_headers),
    };
    let graphql = warp::path::end().and(
        handler::graphql_request(handler_config.clone())
            .or(handler::graphql_websocket(handler_config.clone()))
            .or(handler::graphql_playground()),
    );
    let health = warp::path!("health").map(|| warp::reply::json(&"healthy"));
    let routes = graphql.or(health);

    let bind_addr: SocketAddr = config
        .bind
        .parse()
        .context(format!("Failed to parse bind addr '{}'", config.bind))?;

    let (addr, server) =
        warp::serve(routes).bind_with_graceful_shutdown(bind_addr, signal::ctrl_c().map(|_| ()));
    tracing::info!(addr = %addr, "Listening");
    server.await;
    tracing::info!("Server shutdown");
    Ok(())
}
