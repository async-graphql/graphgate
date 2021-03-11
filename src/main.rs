mod config;
mod graphql_filter;
mod k8s;
mod options;
mod shared_route_table;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use structopt::StructOpt;
use tokio::time::Duration;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};
use warp::Filter;

use config::Config;
use options::Options;
use shared_route_table::SharedRouteTable;

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

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let options: Options = Options::from_args();

    let config = Arc::new(
        toml::from_str::<Config>(
            &std::fs::read_to_string(&options.config)
                .with_context(|| format!("Failed to load config file '{}'", options.config))?,
        )
        .with_context(|| format!("Failed to parse config file '{}'", options.config))?,
    );

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

    let graphql =
        warp::path::end().and(graphql_filter::graphql(shared_route_table, config.clone()));

    let health = warp::path!("health").map(|| warp::reply::json(&"healthy"));
    let routes = graphql.or(health);

    let bind_addr: SocketAddr = config
        .bind
        .parse()
        .context(format!("Failed to parse bind addr '{}'", config.bind))?;
    tracing::info!(addr = %bind_addr, "Listening");
    warp::serve(routes).bind(bind_addr).await;
    Ok(())
}
