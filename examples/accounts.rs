use std::convert::Infallible;

use async_graphql::{EmptyMutation, Object, Schema, SimpleObject, Subscription, ID};
use async_graphql_warp::{graphql, graphql_subscription};
use futures_util::stream::Stream;
use tokio::time::Duration;
use warp::{Filter, Reply};

#[derive(SimpleObject)]
struct User {
    id: ID,
    username: String,
}

struct Query;

#[Object(extends)]
impl Query {
    /// Get the current user.
    async fn me(&self) -> User {
        User {
            id: "1234".into(),
            username: "Me".to_string(),
        }
    }

    #[graphql(entity)]
    async fn find_user_by_id(&self, id: ID) -> User {
        let username = if id == "1234" {
            "Me".to_string()
        } else {
            format!("User {:?}", id)
        };
        User { id, username }
    }
}

struct Subscription;

#[Subscription(extends)]
impl Subscription {
    async fn users(&self) -> impl Stream<Item = User> {
        async_stream::stream! {
            loop {
                tokio::time::sleep(Duration::from_secs(fastrand::u64(1..3))).await;
                yield User { id: "1234".into(), username: "Me".to_string() };
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let schema = Schema::build(Query, EmptyMutation, Subscription)
        .extension(async_graphql::extensions::ApolloTracing)
        .enable_subscription_in_federation()
        .finish();

    let routes = graphql(schema.clone())
        .and(warp::post())
        .and_then(
            |(schema, request): (
                Schema<Query, EmptyMutation, Subscription>,
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
