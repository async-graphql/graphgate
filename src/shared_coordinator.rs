use std::sync::Arc;

use anyhow::{Context, Error, Result};
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use graphgate_core::{
    ComposedSchema, Coordinator, Executor, PlanBuilder, Request, Response, ServerError,
};
use graphgate_transports::CoordinatorImpl;
use serde::Deserialize;
use tokio::sync::{mpsc, Mutex};
use tokio::time::Duration;
use value::ConstValue;
use warp::http::{Response as HttpResponse, StatusCode};

enum Command {
    ChangeCoordinator(CoordinatorImpl),
}

struct InnerSharedCoordinator {
    schema: Option<Arc<ComposedSchema>>,
    coordinator: Option<Arc<CoordinatorImpl>>,
}

#[derive(Clone)]
pub struct SharedCoordinator {
    inner: Arc<Mutex<InnerSharedCoordinator>>,
    tx: mpsc::UnboundedSender<Command>,
}

impl Default for SharedCoordinator {
    fn default() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let coordinator = Self {
            inner: Arc::new(Mutex::new(InnerSharedCoordinator {
                schema: None,
                coordinator: None,
            })),
            tx,
        };
        tokio::spawn({
            let coordinator = coordinator.clone();
            async move { coordinator.update_loop(rx).await }
        });
        coordinator
    }
}

impl SharedCoordinator {
    pub fn with_coordinator(coordinator: CoordinatorImpl) -> Self {
        let c = Self::default();
        c.set_coordinator(coordinator);
        c
    }

    async fn update_loop(self, mut rx: mpsc::UnboundedReceiver<Command>) {
        let mut update_interval = tokio::time::interval(Duration::from_secs(30));
        tokio::time::sleep(Duration::from_secs(5)).await;

        loop {
            tokio::select! {
                _ = update_interval.tick() => {
                    if let Err(err) = self.update().await {
                        tracing::error!(error = %err, "Failed to update schema.");
                    }
                }
                command = rx.recv() => {
                    if let Some(command) = command {
                        match command {
                            Command::ChangeCoordinator(coordinator) => {
                                let mut inner = self.inner.lock().await;
                                inner.coordinator = Some(Arc::new(coordinator));
                                inner.schema = None;
                            }
                        }
                    }
                }
            }
        }
    }

    async fn update(&self) -> Result<()> {
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

        let coordinator = match self.inner.lock().await.coordinator.clone() {
            Some(coordinator) => coordinator,
            None => return Ok(()),
        };

        let services = coordinator.services();
        let resp = futures_util::future::try_join_all(services.iter().map(|service| {
            let coordinator = coordinator.clone();
            async move {
                let resp = coordinator
                    .query(&service, Request::new(QUERY_SDL))
                    .await
                    .with_context(|| format!("Failed to fetch SDL from '{}'.", service))?;
                let resp: ResponseQuery =
                    value::from_value(resp.data).context("Failed to parse response.")?;
                let document = parser::parse_schema(resp.service.sdl)
                    .with_context(|| format!("Invalid SDL from '{}'.", service))?;
                Ok::<_, Error>((service.to_string(), document))
            }
        }))
        .await?;

        let schema = ComposedSchema::combine(resp)?;
        self.inner.lock().await.schema = Some(Arc::new(schema));
        Ok(())
    }

    pub fn set_coordinator(&self, coordinator: CoordinatorImpl) {
        self.tx.send(Command::ChangeCoordinator(coordinator)).ok();
    }

    pub async fn query(&self, request: Request) -> HttpResponse<String> {
        let (composed_schema, coordinator) = {
            let inner = self.inner.lock().await;
            (inner.schema.clone(), inner.coordinator.clone())
        };

        let document = match parser::parse_query(request.query) {
            Ok(document) => document,
            Err(err) => {
                return HttpResponse::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(err.to_string())
                    .unwrap();
            }
        };

        let (composed_schema, coordinator) = match (composed_schema, coordinator) {
            (Some(composed_schema), Some(coordinator)) => (composed_schema, coordinator),
            _ => {
                return HttpResponse::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(
                        serde_json::to_string(&Response {
                            data: ConstValue::Null,
                            errors: vec![ServerError {
                                message: "Not ready.".to_string(),
                                locations: Default::default(),
                            }],
                        })
                        .unwrap(),
                    )
                    .unwrap()
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
                return HttpResponse::builder()
                    .status(StatusCode::OK)
                    .body(serde_json::to_string(&response).unwrap())
                    .unwrap();
            }
        };
        let executor = Executor::new(&composed_schema, &*coordinator);
        HttpResponse::builder()
            .status(StatusCode::OK)
            .body(serde_json::to_string(&executor.execute(&plan).await).unwrap())
            .unwrap()
    }

    pub async fn subscribe(&self, request: Request) -> BoxStream<'static, Response> {
        let coordinator = self.clone();
        let stream = async_stream::stream! {
            let (composed_schema, coordinator) = {
                let inner = coordinator.inner.lock().await;
                (inner.schema.clone(), inner.coordinator.clone())
            };

            let (composed_schema, coordinator) = match (composed_schema, coordinator) {
                (Some(composed_schema), Some(coordinator)) => (composed_schema, coordinator),
                _ => {
                    yield Response {
                        errors: vec![ServerError {
                            message: "Not ready.".to_string(),
                            locations: Default::default(),
                        }],
                        ..Response::default()
                    };
                    return;
                }
            };

            let document = match parser::parse_query(request.query) {
                Ok(document) => document,
                Err(err) => {
                    yield Response {
                        errors: vec![ServerError {
                            message: err.to_string(),
                            locations: Default::default(),
                        }],
                        ..Response::default()
                    };
                    return;
                }
            };
            let mut plan_builder = PlanBuilder::new(&composed_schema, document).variables(request.variables);
            if let Some(operation) = request.operation {
                plan_builder = plan_builder.operation_name(operation);
            }
            let plan = match plan_builder.plan_subscribe() {
                Ok(plan) => plan,
                Err(response) => {
                    yield response;
                    return;
                }
            };
            let executor = Executor::new(&composed_schema, &*coordinator);
            let mut stream = executor.execute_stream(&plan).await;
            while let Some(resp) = stream.next().await {
                yield resp;
            }
        };
        Box::pin(stream)
    }
}
