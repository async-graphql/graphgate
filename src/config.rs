use graphgate_handler::{ServiceRoute, ServiceRouteTable};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default = "default_bind")]
    pub bind: String,

    #[serde(default)]
    pub gateway_name: String,

    #[serde(default)]
    pub services: Vec<ServiceConfig>,

    #[serde(default)]
    pub forward_headers: Vec<String>,

    pub jaeger: Option<JaegerConfig>,

    pub cors: Option<CorsConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServiceConfig {
    pub name: String,
    pub addr: String,
    #[serde(default)]
    pub tls: bool,
    pub query_path: Option<String>,
    pub subscribe_path: Option<String>,
    pub introspection_path: Option<String>,
    pub websocket_path: Option<String>,
}

impl ServiceConfig {
    // websocket path should default to query path unless set
    fn default_or_set_websocket_path(&self) -> Option<String> {
        if self.websocket_path.is_some() {
            self.websocket_path.clone()
        } else {
            self.query_path.clone()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CorsConfig {
    pub allow_any_origin: Option<bool>,
    pub allow_methods: Option<Vec<String>>,
    pub allow_credentials: Option<bool>,
    pub allow_headers: Option<Vec<String>>,
    pub allow_origins: Option<Vec<String>>,
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
                    introspection_path: service.introspection_path.clone(),
                    websocket_path: service.default_or_set_websocket_path(),
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
