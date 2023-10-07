use std::convert::Infallible;

use async_graphql::{Context, EmptyMutation, Object, Schema, SimpleObject, Subscription};
use async_graphql_warp::{graphql, graphql_subscription};
use futures_util::stream::Stream;
use tokio::time::Duration;
use warp::{Filter, Reply};

#[derive(SimpleObject, Clone)]
struct Product {
    upc: String,
    name: String,
    price: i32,
}

struct Query;

#[Object(extends)]
impl Query {
    async fn top_products<'a>(&self, ctx: &'a Context<'_>) -> &'a Vec<Product> {
        ctx.data_unchecked::<Vec<Product>>()
    }

    #[graphql(entity)]
    async fn find_product_by_upc<'a>(&self, ctx: &Context<'a>, upc: String) -> Option<&'a Product> {
        let hats = ctx.data_unchecked::<Vec<Product>>();
        hats.iter().find(|product| product.upc == upc)
    }
}

struct Subscription;

#[Subscription(extends)]
impl Subscription {
    async fn products(&self) -> impl Stream<Item = Product> {
        async_stream::stream! {
            loop {
                tokio::time::sleep(Duration::from_secs(fastrand::u64(5..10))).await;
                yield Product {
                    upc: "top-1".to_string(),
                    name: "Trilby".to_string(),
                    price: 11,
                };
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let hats = vec![
        Product {
            upc: "top-1".to_string(),
            name: "Trilby".to_string(),
            price: 11,
        },
        Product {
            upc: "top-2".to_string(),
            name: "Fedora".to_string(),
            price: 22,
        },
        Product {
            upc: "top-3".to_string(),
            name: "Boater".to_string(),
            price: 33,
        },
    ];

    let schema = Schema::build(Query, EmptyMutation, Subscription)
        .extension(async_graphql::extensions::ApolloTracing)
        .enable_subscription_in_federation()
        .data(hats)
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

    warp::serve(routes).run(([0, 0, 0, 0], 8002)).await;
}
