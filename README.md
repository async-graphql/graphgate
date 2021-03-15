# GraphGate

<div align="center">
  <!-- CI -->
  <img src="https://github.com/async-graphql/graphgate/workflows/CI/badge.svg" />
  <!-- codecov -->
  <img src="https://codecov.io/gh/async-graphql/graphgate/branch/master/graph/badge.svg" />
  <a href="https://github.com/rust-secure-code/safety-dance/">
    <img src="https://img.shields.io/badge/unsafe-forbidden-success.svg?style=flat-square"
      alt="Unsafe Rust forbidden" />
  </a>
</div>

GraphGate is a [Apollo Federation](https://www.apollographql.com/apollo-federation) implemented in Rust.

## Quick start

A GraphQL API composed of 3 services (accounts, products, reviews).

```shell
docker run -p 8000:8000 scott829/graphgate-standalone-demo:latest
```

Open browser [http://localhost:8000](http://localhost:8000)

### Execute query

```graphql
{
    topProducts {
        upc name price reviews {
            body
            author {
                id
                username
            }
        } 
    }
}
```

### Execute subscription

```graphql
subscription {
    users {
        id username reviews {
            body
        }
    }
}
```

## FAQ

### What does Apollo Federation do?

To get the most out of GraphQL, your organization should expose a single data graph that provides a unified interface for querying any combination of your backing data sources. However, it can be challenging to represent an enterprise-scale data graph with a single, monolithic GraphQL server.

To remedy this, you can divide your graph's implementation across multiple composable services with Apollo Federation:

Unlike other distributed GraphQL architectures (such as schema stitching), Apollo Federation uses a declarative programming model that enables each implementing service to implement only the part of your graph that it's responsible for.

### Why use Rust to implement it?

Rust is my favorite programming language. It is safe and fast, and is very suitable for developing API gateway.

### What is the difference between GraphGate and Apollo Federaion?

I guess the performance of GraphGate will be much better (I haven't done benchmarking yet, but will add it soon), and it supports subscription.
