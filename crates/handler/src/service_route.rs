use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use graphgate_planner::{Request, Response};
use http::HeaderMap;

/// Service routing information.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct ServiceRoute {
    /// Service address
    ///
    /// For example: 1.2.3.4:8000, example.com:8080
    pub addr: String,

    /// GraphQL HTTP path, default is `/`.
    pub query_path: Option<String>,

    /// GraphQL WebSocket path, default is `/`.
    pub subscribe_path: Option<String>,
}

/// Service routing table
///
/// The key is the service name.
#[derive(Default, Debug, Clone, Eq, PartialEq)]
pub struct ServiceRouteTable(HashMap<String, ServiceRoute>);

impl Deref for ServiceRouteTable {
    type Target = HashMap<String, ServiceRoute>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ServiceRouteTable {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl ServiceRouteTable {
    /// Call the GraphQL query of the specified service.
    pub async fn query(
        &self,
        service: impl AsRef<str>,
        request: Request,
        header_map: Option<&HeaderMap>,
    ) -> anyhow::Result<Response> {
        let service = service.as_ref();
        let route = self.0.get(service).ok_or_else(|| {
            anyhow::anyhow!("Service '{}' is not defined in the routing table.", service)
        })?;
        let url = match &route.query_path {
            Some(path) => format!("http://{}{}", route.addr, path),
            None => format!("http://{}", route.addr),
        };
        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .headers(header_map.cloned().unwrap_or_default())
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json::<Response>()
            .await?;
        Ok(resp)
    }
}
