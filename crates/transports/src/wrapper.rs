use anyhow::Error;
use graphgate_core::{Request, Response};

use crate::transport::Transport;

pub struct TransportWrapper<T>(pub T);

#[async_trait::async_trait]
impl<T: Transport> Transport for TransportWrapper<T> {
    type Error = Error;

    async fn is_ready(&self) -> bool {
        self.0.is_ready().await
    }

    async fn query(&self, request: Request) -> Result<Response, Self::Error> {
        self.0
            .query(request)
            .await
            .map_err(|err| anyhow::anyhow!("{}", err))
    }
}
