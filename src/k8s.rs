use std::collections::HashMap;

use anyhow::{Context, Result};
use graphgate_transports::CoordinatorImpl;
use k8s_openapi::api::core::v1::Service;
use kube::api::{ListParams, ObjectMeta};
use kube::{Api, Client};

const LABEL_GRAPHQL_SERVICE: &str = "graphgate.org/service";

fn get_label_value<'a>(meta: &'a ObjectMeta, name: &str) -> Option<&'a str> {
    meta.labels
        .iter()
        .flatten()
        .find(|(key, _)| key.as_str() == name)
        .map(|(_, value)| value.as_str())
}

pub async fn find_graphql_services() -> Result<HashMap<String, String>> {
    let client = Client::try_default()
        .await
        .context("Failed to create kube client.")?;
    let namespace = std::env::var("NAMESPACE").unwrap_or("default".into());
    let mut graphql_services = HashMap::new();
    let services_api: Api<Service> = Api::namespaced(client, &namespace);
    let services = services_api
        .list(&ListParams::default().labels(LABEL_GRAPHQL_SERVICE))
        .await?;

    for service in &services {
        if let Some((host, service_name)) = service
            .metadata
            .name
            .as_deref()
            .zip(get_label_value(&service.metadata, LABEL_GRAPHQL_SERVICE))
        {
            for service_port in service
                .spec
                .iter()
                .map(|spec| spec.ports.iter())
                .flatten()
                .flatten()
            {
                if let Some(protocol) = service_port.protocol.as_deref() {
                    if matches!(protocol, "http" | "https" | "ws" | "wss") {
                        graphql_services.insert(
                            service_name.to_string(),
                            format!("{}://{}:{}", protocol, host, service_port.port),
                        );
                    }
                }
            }
        }
    }

    Ok(graphql_services)
}

pub fn create_coordinator(graphql_services: &HashMap<String, String>) -> Result<CoordinatorImpl> {
    let mut coordinator = CoordinatorImpl::default();
    for (service, url) in graphql_services {
        coordinator = coordinator.add_url(service, url)?;
    }
    Ok(coordinator)
}
