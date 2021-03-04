mod config;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Error, Result};
use clap::{crate_version, App, Arg};
use graphgate_core::{ComposedSchema, Coordinator, Executor, PlanBuilder};
use graphgate_transports::CoordinatorImpl;
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::time::Duration;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};
use value::Variables;
use warp::http::{Response as HttpResponse, StatusCode};
use warp::Filter;

use config::{Config, ServiceConfig};

type SharedComposedSchema = Arc<Mutex<Option<Arc<ComposedSchema>>>>;

#[derive(Debug, Deserialize)]
struct Request {
    query: String,
    operation: Option<String>,
    #[serde(default)]
    variables: Variables,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let matches = App::new("GraphQL Gate")
        .version(crate_version!())
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .value_name("FILE")
                .default_value("config.toml")
                .help("Sets a custom config file")
                .takes_value(true),
        )
        .get_matches();
    let config_file = matches.value_of("config").unwrap();
    let config: Config = toml::from_str(
        &std::fs::read_to_string(config_file).context("Failed to load config file.")?,
    )
    .context("Failed to parse config file.")?;

    let coordinator = Arc::new(
        config
            .create_coordinator()
            .context("Failed to create coordinator.")?,
    );
    let shared_composed_schema: SharedComposedSchema = Default::default();
    start_update_schema_loop(
        shared_composed_schema.clone(),
        coordinator.clone(),
        config.services.clone(),
    );
    serve(config, shared_composed_schema.clone(), coordinator.clone()).await?;
    Ok(())
}

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

fn start_update_schema_loop(
    shared_composed_schema: SharedComposedSchema,
    coordinator: Arc<CoordinatorImpl>,
    services: Vec<ServiceConfig>,
) {
    tokio::spawn(async move {
        loop {
            tracing::debug!("Update schema.");
            match update_schema(&coordinator, &services).await {
                Ok(schema) => *shared_composed_schema.lock().await = Some(Arc::new(schema)),
                Err(err) => tracing::error!(error = %err, "Failed to update schema"),
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

async fn update_schema(
    coordinator: &impl Coordinator<Error = Error>,
    services: &[ServiceConfig],
) -> Result<ComposedSchema> {
    const QUERY_SDL: &str = "{ _service { sdl }}";

    #[derive(Deserialize)]
    struct ResponseQuery {
        #[serde(rename = "_service")]
        service: ResponseService,
    }

    #[derive(Deserialize)]
    struct ResponseService {
        sdl: String,
    }

    let resp = futures_util::future::try_join_all(services.iter().map(|service| async move {
        let resp = coordinator
            .query(&service.name, QUERY_SDL, Default::default())
            .await
            .context(format!("Failed to fetch SDL from '{}'.", service.name))?;
        let resp: ResponseQuery =
            value::from_value(resp.data).context("Failed to parse response.")?;
        let document = parser::parse_schema(resp.service.sdl)
            .context(format!("Invalid SDL from '{}'.", service.name))?;
        Ok::<_, Error>((service.name.clone(), document))
    }))
    .await?;

    Ok(ComposedSchema::combine(resp).context("Unable to merge schema.")?)
}

async fn serve(
    config: Config,
    shared_composed_schema: SharedComposedSchema,
    coordinator: Arc<CoordinatorImpl>,
) -> Result<()> {
    let bind_addr: SocketAddr = config
        .bind
        .parse()
        .context(format!("Failed to parse bind addr '{}'.", config.bind))?;

    let graphql = warp::path::end()
        .and(warp::post())
        .and(warp::body::json())
        .and_then({
            let shared_composed_schema = shared_composed_schema.clone();
            let coordinator = coordinator.clone();
            move |request: Request| {
                let shared_composed_schema = shared_composed_schema.clone();
                let coordinator = coordinator.clone();
                async move {
                    let composed_schema = {
                        let shared_composed_schema = shared_composed_schema.lock().await;
                        match &*shared_composed_schema {
                            Some(composed_schema) => composed_schema.clone(),
                            None => {
                                return Ok(HttpResponse::builder()
                                    .status(StatusCode::SERVICE_UNAVAILABLE)
                                    .body("Gateway is not ready.".to_string()));
                            }
                        }
                    };
                    let document = match parser::parse_query(request.query) {
                        Ok(document) => document,
                        Err(err) => {
                            return Ok(HttpResponse::builder()
                                .status(StatusCode::BAD_REQUEST)
                                .body(err.to_string()));
                        }
                    };
                    let mut plan_builder =
                        PlanBuilder::new(&composed_schema, document).variables(request.variables);
                    if let Some(operation) = request.operation {
                        plan_builder = plan_builder.operation_name(operation);
                    }
                    let plan = match plan_builder.plan() {
                        Ok(plan) => plan,
                        Err(response) => {
                            return Ok(HttpResponse::builder()
                                .status(StatusCode::OK)
                                .body(serde_json::to_string(&response).unwrap()))
                        }
                    };
                    let executor = Executor::new(&composed_schema, coordinator);
                    Ok::<_, std::convert::Infallible>(
                        HttpResponse::builder()
                            .status(StatusCode::OK)
                            .body(serde_json::to_string(&executor.execute(&plan).await).unwrap()),
                    )
                }
            }
        });

    let graphql_playground = warp::path::end().and(warp::get()).map(|| {
        HttpResponse::builder()
            .header("content-type", "text/html")
            .body(include_str!("playground.html"))
    });

    tracing::info!(addr = %bind_addr, "Listen");
    let routes = graphql.or(graphql_playground);
    warp::serve(routes).run(bind_addr).await;
    Ok(())
}
