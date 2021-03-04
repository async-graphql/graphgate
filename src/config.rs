use anyhow::Result;
use graphgate_transports::CoordinatorImpl;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServiceConfig {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub services: Vec<ServiceConfig>,
    #[serde(default = "default_bind")]
    pub bind: String,
}

impl Config {
    pub fn create_coordinator(&self) -> Result<CoordinatorImpl> {
        let mut coordinator = CoordinatorImpl::default();
        for service in &self.services {
            coordinator = coordinator.add_url(&service.name, &service.url)?;
        }
        Ok(coordinator)
    }
}

fn default_bind() -> String {
    "127.0.0.1:8000".to_string()
}
