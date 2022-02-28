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
use opentelemetry::global::GlobalTracerProvider;
use opentelemetry::trace::noop::NoopTracerProvider;
use opentelemetry_prometheus::PrometheusExporter;
use prometheus::{Encoder, TextEncoder};
use structopt::StructOpt;
use tokio::signal;
use tokio::time::Duration;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};
use warp::http::Response as HttpResponse;
use warp::hyper::StatusCode;
use warp::{Filter, Rejection, Reply};

use config::Config;
use options::Options;

// Use Jemalloc only for musl-64 bits platforms
#[cfg(all(target_env = "musl", target_pointer_width = "64"))]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

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

async fn update_route_table_in_k8s(shared_route_table: SharedRouteTable, gateway_name: String) {
    let mut prev_route_table = None;
    loop {
        match k8s::find_graphql_services(&gateway_name).await {
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

fn init_tracer(config: &Config) -> Result<GlobalTracerProvider> {
    let uninstall = match &config.jaeger {
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
    Ok(uninstall)
}

pub fn metrics(
    exporter: PrometheusExporter,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    warp::path!("metrics").and(warp::get()).map({
        move || {
            let mut buffer = Vec::new();
            let encoder = TextEncoder::new();
            let metric_families = exporter.registry().gather();
            if let Err(err) = encoder.encode(&metric_families, &mut buffer) {
                return HttpResponse::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(err.to_string().into_bytes())
                    .unwrap();
            }
            HttpResponse::builder()
                .status(StatusCode::OK)
                .body(buffer)
                .unwrap()
        }
    })
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
    let _uninstall = init_tracer(&config)?;
    let exporter = opentelemetry_prometheus::exporter().init();

    let mut shared_route_table = SharedRouteTable::default();
    if !config.services.is_empty() {
        tracing::info!("Route table in the configuration file.");
        shared_route_table.set_route_table(config.create_route_table());
        shared_route_table.set_receive_headers(config.receive_headers);
    } else if std::env::var("KUBERNETES_SERVICE_HOST").is_ok() {
        tracing::info!("Route table within the current namespace in Kubernetes cluster.");
        shared_route_table.set_receive_headers(config.receive_headers);
        tokio::spawn(update_route_table_in_k8s(
            shared_route_table.clone(),
            config.gateway_name.clone(),
        ));
    } else {
        tracing::info!("Route table is empty.");
        return Ok(());
    }

    let handler_config = HandlerConfig {
        shared_route_table,
        forward_headers: Arc::new(config.forward_headers),
    };

    let cors = if let Some(cors_config) = config.cors {
        let warp_cors = warp::cors();

        let origins_vec = cors_config.allow_origins.unwrap_or_default();

        let origins: Vec<&str> = origins_vec.iter().map(|s| s as &str).collect();

        let headers_vec = cors_config.allow_headers.unwrap_or_default();

        let headers: Vec<&str> = headers_vec.iter().map(|s| s as &str).collect();

        let allow_credentials = cors_config.allow_credentials.unwrap_or(false);

        let allow_methods_vec = cors_config.allow_methods.unwrap_or_default();

        let methods: Vec<&str> = allow_methods_vec.iter().map(|s| s as &str).collect();

        let cors_setup = warp_cors
            .allow_headers(headers)
            .allow_origins(origins)
            .allow_methods(methods)
            .allow_credentials(allow_credentials);

        if let Some(true) = cors_config.allow_any_origin {
            Some(cors_setup.allow_any_origin())
        } else {
            Some(cors_setup)
        }
    } else {
        None
    };

    let graphql = warp::path::end().and(
        handler::graphql_request(handler_config.clone())
            .or(handler::graphql_websocket(handler_config.clone()))
            .or(handler::graphql_playground()),
    );
    let health = warp::path!("health").map(|| warp::reply::json(&"healthy"));

    let bind_addr: SocketAddr = config
        .bind
        .parse()
        .context(format!("Failed to parse bind addr '{}'", config.bind))?;
    if let Some(warp_cors) = cors {
        let routes = graphql.or(health).or(metrics(exporter)).with(warp_cors);
        let (addr, server) = warp::serve(routes)
            .bind_with_graceful_shutdown(bind_addr, signal::ctrl_c().map(|_| ()));
        tracing::info!(addr = %addr, "Listening");
        server.await;
        tracing::info!("Server shutdown");
    } else {
        let routes = graphql.or(health).or(metrics(exporter));
        let (addr, server) = warp::serve(routes)
            .bind_with_graceful_shutdown(bind_addr, signal::ctrl_c().map(|_| ()));
        tracing::info!(addr = %addr, "Listening");
        server.await;
        tracing::info!("Server shutdown");
    }

    Ok(())
}
