# GraphGate 是什么?

GraphGate 是一个用 Rust 语言实现的 [Apollo Federation](https://www.apollographql.com/apollo-federation) 网关

# Apollo Federation 是做什么的？

在微服务架构中数据可能位于不同的位置，把多个服务提供的 API 合并到一起是一件有挑战的事情。

为了解决这个问题，你可以使用 Federation 将API的实现划分为多个可组合服务：

与其他分布式 GraphQL 结构（例如模式缝合）不同，Federation 使用声明性编程模型，该模型使每个服务仅实现图中负责的部分。

# 为什么要用 Rust 实现它？

Rust是我最喜欢的编程语言，它安全并且快速，非常适合用于开发API网关这样的基础服务。

# GraphGate和Apollo Federation的主要区别是什么？

我猜GraphGate的性能会好很多（我还没有做基准测试，但很快会加上），并且支持订阅。