use anyhow::Error;
use graphgate_core::Response;
use value::Variables;

use crate::transport::Transport;

pub struct TransportWrapper<T>(pub T);

#[async_trait::async_trait]
impl<T: Transport> Transport for TransportWrapper<T> {
    type Error = Error;

    async fn query(&self, query: &str, variables: Variables) -> Result<Response, Self::Error> {
        self.0
            .query(query, variables)
            .await
            .map_err(|err| anyhow::anyhow!("{}", err))
    }
}
