use once_cell::sync::Lazy;
use opentelemetry::{
    global,
    metrics::{Counter, Histogram},
};

pub struct Metrics {
    pub query_counter: Counter<u64>,
    pub query_histogram: Histogram<f64>,
}

pub static METRICS: Lazy<Metrics> = Lazy::new(|| {
    let meter = global::meter("graphgate");
    let query_counter = meter
        .u64_counter("graphgate.queries_total")
        .with_description("Total number of GraphQL queries executed")
        .init();
    let query_histogram = meter
        .f64_histogram("graphgate.graphql_query_duration_seconds")
        .with_description("The GraphQL query latencies in seconds.")
        .init();
    Metrics {
        query_counter,
        query_histogram,
    }
});
