mod config;
mod http;
mod k8s;
mod options;
mod shared_coordinator;

use std::net::SocketAddr;

use anyhow::{Context, Result};
use graphgate_core::ComposedSchema;
use graphgate_transports::{CoordinatorImpl, RoundRobinTransport};
use structopt::StructOpt;
use tokio::time::Duration;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};
use warp::Filter;

use config::Config;
use http::graphql_filter;
use options::Options;
use shared_coordinator::SharedCoordinator;

fn init_tracing() {
    tracing_subscriber::registry()
        .with(fmt::layer().with_target(true))
        .with(
            EnvFilter::try_from_default_env()
                .or_else(|_| EnvFilter::try_new("info"))
                .unwrap(),
        )
        .init();
}

async fn start_with_config_file(config_file: String) -> Result<()> {
    let config: Config = toml::from_str(
        &std::fs::read_to_string(&config_file)
            .with_context(|| format!("Failed to load config file '{}'", config_file))?,
    )
    .with_context(|| format!("Failed to parse config file '{}'", config_file))?;

    let coordinator = SharedCoordinator::with_coordinator(
        config
            .create_coordinator()
            .context("Failed to create coordinator")?,
    );
    let bind_addr: SocketAddr = config
        .bind
        .parse()
        .context(format!("Failed to parse bind addr '{}'", config.bind))?;
    warp::serve(graphql_filter(coordinator))
        .bind(bind_addr)
        .await;
    Ok(())
}

async fn start_with_schema_file(schema_file: String, bind: String) -> Result<()> {
    let schema: ComposedSchema = ComposedSchema::parse(
        &std::fs::read_to_string(&schema_file)
            .with_context(|| format!("Failed to load schema file '{}'", schema_file))?,
    )
    .with_context(|| format!("Failed to parse config file '{}'", schema_file))?;

    let mut coordinator = CoordinatorImpl::default();
    for (service, urls) in &schema.services {
        let mut transport = RoundRobinTransport::default();
        for url in urls {
            transport = transport
                .add_url(url)
                .context(format!("Invalid service url '{}'", url))?;
        }
        coordinator = coordinator.add(service, transport);
    }

    let bind_addr: SocketAddr = bind
        .parse()
        .context(format!("Failed to parse bind addr '{}'", bind))?;

    warp::serve(graphql_filter(SharedCoordinator::with_coordinator(
        coordinator,
    )))
    .bind(bind_addr)
    .await;
    Ok(())
}

async fn start_in_k8s(bind: String) -> Result<()> {
    let shared_coordinator = SharedCoordinator::default();
    tokio::spawn({
        let shared_coordinator = shared_coordinator.clone();
        async move {
            let mut prev_services = None;

            loop {
                match k8s::find_graphql_services().await {
                    Ok(services) => {
                        if Some(&services) != prev_services.as_ref() {
                            match k8s::create_coordinator(&services) {
                                Ok(coordinator) => {
                                    shared_coordinator.set_coordinator(coordinator);
                                    prev_services = Some(services);
                                }
                                Err(err) => {
                                    tracing::error!(error = %err, "Failed to create coordinator.");
                                }
                            }
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

    let bind_addr: SocketAddr = bind
        .parse()
        .context(format!("Failed to parse bind addr '{}'", bind))?;

    let graphql = warp::path::end().and(http::graphql_filter(shared_coordinator));
    let health = warp::path!("health").map(|| warp::reply::json(&"healthy"));
    let routes = graphql.or(health);
    warp::serve(routes).bind(bind_addr).await;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let options: Options = Options::from_args();

    match options {
        Options::Serve { config } => start_with_config_file(config).await,
        Options::Schema { schema, bind } => start_with_schema_file(schema, bind).await,
        Options::Controller { bind } => start_in_k8s(bind).await,
    }
}
