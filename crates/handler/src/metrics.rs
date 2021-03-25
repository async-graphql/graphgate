use once_cell::sync::Lazy;
use opentelemetry::global;
use opentelemetry::metrics::{BoundCounter, BoundValueRecorder};

pub struct Metrics {
    pub query_counter: BoundCounter<'static, u64>,
    pub query_histogram: BoundValueRecorder<'static, f64>,
}

pub static METRICS: Lazy<Metrics> = Lazy::new(|| {
    let meter = global::meter("graphgate");
    let query_counter = meter
        .u64_counter("graphgate.queries_total")
        .with_description("Total number of GraphQL queries executed")
        .init()
        .bind(&[]);
    let query_histogram = meter
        .f64_value_recorder("graphgate.graphql_query_duration_seconds")
        .with_description("The GraphQL query latencies in seconds.")
        .init()
        .bind(&[]);
    Metrics {
        query_counter,
        query_histogram,
    }
});
