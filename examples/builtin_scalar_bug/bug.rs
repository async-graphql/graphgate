use std::convert::Infallible;

use async_graphql::{
    Context, EmptyMutation, EmptySubscription, Object, Schema, SimpleObject, Subscription, ID,
};
use async_graphql_warp::{graphql, graphql_subscription};
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use tokio::time::Duration;
use warp::{Filter, Reply};

// Run the service:
// ```
// $ cargo run --example builtin_scalar_bug
// ```
//
// Run the gateway:
// ```
// $ cargo run -- ./examples/builtin_scalar_bug/config.toml
// ```
//
// Query the service directly:
// ```
// $ curl -s 'http://localhost:8001' --data-binary '{"query":"{ builtinScalar customScalar }\n"}' | jq .data
// {
//   "builtinScalar": "Hi, I'm builtin",
//   "customScalar": "Hi, I'm custom"
// }
// ```
//
// Run the same query through the gateway:
// ```
// $ curl -s 'http://localhost:8000' --data-binary '{"query":"{ builtinScalar customScalar }\n"}' | jq .data
// {
//   "builtinScalar": null,
//   "customScalar": "Hi, I'm custom"
// }
// ```
//
// :(

#[derive(Serialize, Deserialize)]
struct CustomString(String);
async_graphql::scalar!(CustomString);

struct Query;

#[Object(extends)]
impl Query {
    async fn builtin_scalar(&self) -> String {
        "Hi, I'm builtin".into()
    }
    async fn custom_scalar(&self) -> CustomString {
        CustomString("Hi, I'm custom".into())
    }

    #[graphql(entity)] // just so we get _service
    async fn find_me(&self, constant: String) -> BuiltinScalarBug {
        BuiltinScalarBug { constant }
    }
}

#[derive(SimpleObject)]
struct BuiltinScalarBug {
    constant: String,
}

#[tokio::main]
async fn main() {
    let schema = Schema::build(Query, EmptyMutation, EmptySubscription).finish();

    let routes = graphql(schema.clone())
        .and(warp::post())
        .and_then(
            |(schema, request): (
                Schema<Query, EmptyMutation, EmptySubscription>,
                async_graphql::Request,
            )| async move {
                Ok::<_, Infallible>(
                    warp::reply::json(&schema.execute(request).await).into_response(),
                )
            },
        )
        .or(graphql_subscription(schema));

    warp::serve(routes).run(([0, 0, 0, 0], 8001)).await;
}
