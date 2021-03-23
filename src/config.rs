use graphgate_handler::{ServiceRoute, ServiceRouteTable};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default = "default_bind")]
    pub bind: String,

    #[serde(default)]
    pub services: Vec<ServiceConfig>,

    #[serde(default)]
    pub forward_headers: Vec<String>,

    pub jaeger: Option<JaegerConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServiceConfig {
    pub name: String,
    pub addr: String,
    #[serde(default)]
    pub tls: bool,
    pub query_path: Option<String>,
    pub subscribe_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct JaegerConfig {
    pub agent_endpoint: String,

    #[serde(default = "default_jaeger_service_name")]
    pub service_name: String,
}

impl Config {
    pub fn create_route_table(&self) -> ServiceRouteTable {
        let mut route_table = ServiceRouteTable::default();
        for service in &self.services {
            route_table.insert(
                service.name.clone(),
                ServiceRoute {
                    addr: service.addr.clone(),
                    tls: service.tls,
                    query_path: service.query_path.clone(),
                    subscribe_path: service.subscribe_path.clone(),
                },
            );
        }
        route_table
    }
}

fn default_bind() -> String {
    "127.0.0.1:8000".to_string()
}

fn default_jaeger_service_name() -> String {
    "graphgate".to_string()
}
