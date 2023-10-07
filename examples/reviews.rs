use std::convert::Infallible;

use async_graphql::{Context, EmptyMutation, Object, Schema, SimpleObject, Subscription, ID};
use async_graphql_warp::{graphql, graphql_subscription};
use futures_util::stream::Stream;
use tokio::time::Duration;
use warp::{Filter, Reply};

struct User {
    id: ID,
}

#[Object(extends)]
impl User {
    #[graphql(external)]
    async fn id(&self) -> &ID {
        &self.id
    }

    async fn reviews<'a>(&self, ctx: &'a Context<'_>) -> Vec<&'a Review> {
        let reviews = ctx.data_unchecked::<Vec<Review>>();
        reviews
            .iter()
            .filter(|review| review.author.id == self.id)
            .collect()
    }
}

struct Product {
    upc: String,
}

#[Object(extends)]
impl Product {
    #[graphql(external)]
    async fn upc(&self) -> &String {
        &self.upc
    }

    async fn reviews<'a>(&self, ctx: &'a Context<'_>) -> Vec<&'a Review> {
        let reviews = ctx.data_unchecked::<Vec<Review>>();
        reviews
            .iter()
            .filter(|review| review.product.upc == self.upc)
            .collect()
    }

    async fn error(&self) -> Result<i32, &str> {
        Err("custom error")
    }
}

#[derive(SimpleObject)]
struct Review {
    body: String,
    author: User,
    product: Product,
}

struct Query;

#[Object]
impl Query {
    #[graphql(entity)]
    async fn find_user_by_id(&self, id: ID) -> User {
        User { id }
    }

    #[graphql(entity)]
    async fn find_product_by_upc(&self, upc: String) -> Product {
        Product { upc }
    }
}

struct Subscription;

#[Subscription(extends)]
impl Subscription {
    async fn reviews(&self) -> impl Stream<Item = Review> {
        async_stream::stream! {
            loop {
                tokio::time::sleep(Duration::from_secs(fastrand::u64(5..10))).await;
                yield Review {
                    body: "A highly effective form of birth control.".into(),
                    author: User { id: "1234".into() },
                    product: Product {
                        upc: "top-1".to_string(),
                    },
                };
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let reviews = vec![
        Review {
            body: "A highly effective form of birth control.".into(),
            author: User { id: "1234".into() },
            product: Product {
                upc: "top-1".to_string(),
            },
        },
        Review {
            body: "Fedoras are one of the most fashionable hats around and can look great with a variety of outfits.".into(),
            author: User { id: "1234".into() },
            product: Product {
                upc: "top-1".to_string(),
            },
        },
        Review {
            body: "This is the last straw. Hat you will wear. 11/10".into(),
            author: User { id: "7777".into() },
            product: Product {
                upc: "top-1".to_string(),
            },
        },
    ];

    let schema = Schema::build(Query, EmptyMutation, Subscription)
        .extension(async_graphql::extensions::ApolloTracing)
        .enable_subscription_in_federation()
        .data(reviews)
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

    warp::serve(routes).run(([0, 0, 0, 0], 8003)).await;
}
