use anyhow::{Context, Result};
use graphgate_handler::{ServiceRoute, ServiceRouteTable};
use k8s_openapi::api::core::v1::Service;
use kube::api::{ListParams, ObjectMeta};
use kube::{Api, Client};

const NAMESPACE_PATH: &str = "/var/run/secrets/kubernetes.io/serviceaccount/namespace";
const LABEL_GRAPHQL_SERVICE: &str = "graphgate.org/service";
const LABEL_GRAPHQL_GATEWAY: &str = "graphgate.org/gateway";
const ANNOTATIONS_TLS: &str = "graphgate.org/tls";
const ANNOTATIONS_QUERY_PATH: &str = "graphgate.org/queryPath";
const ANNOTATIONS_SUBSCRIBE_PATH: &str = "graphgate.org/subscribePath";
const ANNOTATIONS_INTROSPECTION_PATH: &str = "graphgate.org/introspectionPath";
const ANNOTATIONS_WEBSOCKET_PATH: &str = "graphgate.org/websocketPath";

fn get_label_value<'a>(meta: &'a ObjectMeta, name: &str) -> Option<&'a str> {
    meta.labels
        .iter()
        .flatten()
        .find(|(key, _)| key.as_str() == name)
        .map(|(_, value)| value.as_str())
}

fn get_annotation_value<'a>(meta: &'a ObjectMeta, name: &str) -> Option<&'a str> {
    meta.annotations
        .iter()
        .flatten()
        .find(|(key, _)| key.as_str() == name)
        .map(|(_, value)| value.as_str())
}

fn get_gateway_or_default(gateway_name: &str) -> String {
    match gateway_name.len() > 0 {
        true => {
            tracing::trace!(
                "Found gateway name: {}. Looking for gateway labels instead.",
                gateway_name
            );
            format!("{}={}", LABEL_GRAPHQL_GATEWAY, gateway_name)
        }
        false => LABEL_GRAPHQL_SERVICE.to_string(),
    }
}

pub async fn find_graphql_services(gateway_name: &str) -> Result<ServiceRouteTable> {
    tracing::trace!("Find GraphQL services.");
    let client = Client::try_default()
        .await
        .context("Failed to create kube client.")?;

    let namespace =
        std::fs::read_to_string(NAMESPACE_PATH).unwrap_or_else(|_| "default".to_string());
    tracing::trace!(namespace = %namespace, "Get current namespace.");

    let mut route_table = ServiceRouteTable::default();
    let services_api: Api<Service> = Api::namespaced(client, &namespace);

    tracing::trace!("List all services.");
    let services = services_api
        .list(&ListParams::default().labels(get_gateway_or_default(gateway_name).as_str()))
        .await
        .context("Failed to call list services api")?;

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
                let tls = get_annotation_value(&service.metadata, ANNOTATIONS_TLS).is_some();
                let query_path = get_annotation_value(&service.metadata, ANNOTATIONS_QUERY_PATH);
                let subscribe_path =
                    get_annotation_value(&service.metadata, ANNOTATIONS_SUBSCRIBE_PATH);
                let introspection_path =
                    get_annotation_value(&service.metadata, ANNOTATIONS_INTROSPECTION_PATH);
                let websocket_path =
                    get_annotation_value(&service.metadata, ANNOTATIONS_WEBSOCKET_PATH);
                route_table.insert(
                    service_name.to_string(),
                    ServiceRoute {
                        addr: format!("{}:{}", host, service_port.port),
                        tls,
                        query_path: query_path.map(ToString::to_string),
                        subscribe_path: subscribe_path.map(ToString::to_string),
                        introspection_path: introspection_path.map(ToString::to_string),
                        websocket_path: websocket_path.map(ToString::to_string),
                    },
                );
            }
        }
    }

    Ok(route_table)
}
