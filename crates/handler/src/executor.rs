use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Duration, Utc};
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use graphgate_planner::{
    FetchNode, FlattenNode, IntrospectionNode, ParallelNode, PathSegment, PlanNode, RootNode,
    SequenceNode, SubscribeNode,
};
use graphgate_planner::{Request, Response, ServerError};
use graphgate_schema::ComposedSchema;
use opentelemetry::trace::{FutureExt, SpanKind, TraceContextExt, Tracer};
use opentelemetry::{global, Context, Key, KeyValue};
use serde::{Deserialize, Deserializer};
use spin::Mutex;
use tokio::sync::mpsc;
use value::{ConstValue, Name, Variables};

use crate::fetcher::{Fetcher, WebSocketFetcher};
use crate::introspection::{IntrospectionRoot, Resolver};
use crate::websocket::WebSocketController;

const KEY_SERVICE: Key = Key::from_static_str("graphgate.service");
const KEY_QUERY: Key = Key::from_static_str("graphgate.query");
const KEY_PATH: Key = Key::from_static_str("graphgate.path");
const KEY_PARENT_TYPE: Key = Key::from_static_str("graphgate.parentType");
const KEY_RETURN_TYPE: Key = Key::from_static_str("graphgate.returnType");
const KEY_FIELD_NAME: Key = Key::from_static_str("graphgate.fieldName");
const KEY_VARIABLES: Key = Key::from_static_str("graphgate.variables");

/// Query plan executor
pub struct Executor<'e> {
    schema: &'e ComposedSchema,
    resp: Mutex<Response>,
}

impl<'e> Executor<'e> {
    pub fn new(schema: &'e ComposedSchema) -> Self {
        Executor {
            schema,
            resp: Mutex::new(Response::default()),
        }
    }

    /// Execute a query plan and return the results.
    ///
    /// Only `Query` and `Mutation` operations are supported.
    pub async fn execute_query(self, fetcher: &impl Fetcher, node: &RootNode<'_>) -> Response {
        let tracer = global::tracer("graphql");
        let span = tracer
            .span_builder("execute")
            .with_kind(SpanKind::Server)
            .start(&tracer);
        let cx = Context::current_with_span(span);

        match node {
            RootNode::Query(node) => {
                self.execute_node(fetcher, node).with_context(cx).await;
                self.resp.into_inner()
            }
            RootNode::Subscribe(_) => Response {
                data: ConstValue::Null,
                errors: vec![ServerError {
                    message: "Not supported".to_string(),
                    locations: Default::default(),
                }],
                extensions: Default::default(),
            },
        }
    }

    /// Execute a subscription plan and return a stream.
    pub async fn execute_stream<'a>(
        self,
        ws_controller: WebSocketController,
        id: &str,
        node: &'a RootNode<'e>,
    ) -> BoxStream<'a, Response> {
        let fetcher = WebSocketFetcher::new(ws_controller.clone());

        match node {
            RootNode::Query(node) => Box::pin(async_stream::stream! {
                self.execute_node(&fetcher, node).await;
                yield self.resp.into_inner();
            }),
            RootNode::Subscribe(SubscribeNode {
                subscribe_nodes,
                flatten_node,
            }) => {
                let (tx, mut rx) = mpsc::unbounded_channel();
                if let Err(err) =
                    futures_util::future::try_join_all(subscribe_nodes.iter().map(|node| {
                        ws_controller.subscribe(
                            id,
                            node.service,
                            Request::new(node.query.to_string())
                                .variables(node.variables.to_variables()),
                            tx.clone(),
                        )
                    }))
                    .await
                {
                    ws_controller.stop(id).await;
                    return futures_util::stream::once(async move {
                        Response {
                            data: ConstValue::Null,
                            errors: vec![ServerError {
                                message: err.to_string(),
                                locations: Default::default(),
                            }],
                            extensions: Default::default(),
                        }
                    })
                    .boxed();
                }

                Box::pin(async_stream::stream! {
                    while let Some(response) = rx.recv().await {
                        if let Some(query_nodes) = flatten_node {
                            *self.resp.lock() = response;
                            self.execute_node(&fetcher, query_nodes).await;
                            yield std::mem::take(&mut *self.resp.lock());
                        } else {
                            yield response;
                        }
                    }
                })
            }
        }
    }

    fn execute_node<'a>(
        &'a self,
        fetcher: &'a impl Fetcher,
        node: &'a PlanNode<'_>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            match node {
                PlanNode::Sequence(sequence) => self.execute_sequence_node(fetcher, sequence).await,
                PlanNode::Parallel(parallel) => self.execute_parallel_node(fetcher, parallel).await,
                PlanNode::Introspection(introspection) => {
                    self.execute_introspection_node(introspection)
                }
                PlanNode::Fetch(fetch) => self.execute_fetch_node(fetcher, fetch).await,
                PlanNode::Flatten(flatten) => self.execute_flatten_node(fetcher, flatten).await,
            }
        })
    }

    async fn execute_sequence_node(&self, fetcher: &impl Fetcher, sequence: &SequenceNode<'_>) {
        for node in &sequence.nodes {
            self.execute_node(fetcher, node).await;
        }
    }

    async fn execute_parallel_node(&self, fetcher: &impl Fetcher, parallel: &ParallelNode<'_>) {
        futures_util::future::join_all(
            parallel
                .nodes
                .iter()
                .map(|node| async move { self.execute_node(fetcher, node).await }),
        )
        .await;
    }

    fn execute_introspection_node(&self, introspection: &IntrospectionNode) {
        let value = tracing::info_span!("Execute introspection node")
            .in_scope(|| IntrospectionRoot.resolve(&introspection.selection_set, self.schema));
        let mut current_resp = self.resp.lock();
        merge_data(&mut current_resp.data, value);
    }

    async fn execute_fetch_node(&self, fetcher: &impl Fetcher, fetch: &FetchNode<'_>) {
        let request = fetch.to_request();

        let tracer = global::tracer("graphql");
        let span = tracer
            .span_builder(&format!("fetch [{}]", fetch.service))
            .with_kind(SpanKind::Server)
            .with_attributes(vec![
                KeyValue::new(KEY_SERVICE, fetch.service.to_string()),
                KeyValue::new(KEY_QUERY, fetch.query.to_string()),
                KeyValue::new(
                    KEY_VARIABLES,
                    serde_json::to_string(&request.variables).unwrap(),
                ),
            ])
            .start(&tracer);
        let cx = Context::current_with_span(span);

        async move {
            let res = fetcher.query(fetch.service, request).await;
            let mut current_resp = self.resp.lock();

            match res {
                Ok(mut resp) => {
                    if resp.errors.is_empty() {
                        add_tracing_spans(&mut resp);
                        merge_data(&mut current_resp.data, resp.data);
                    } else {
                        merge_errors(&mut current_resp.errors, resp.errors);
                    }
                }
                Err(err) => current_resp.errors.push(ServerError {
                    message: err.to_string(),
                    locations: Default::default(),
                }),
            }
        }
        .with_context(cx)
        .await
    }

    async fn execute_flatten_node(&self, fetcher: &impl Fetcher, flatten: &FlattenNode<'_>) {
        enum Representation {
            Keys(ConstValue),
            Skip,
        }

        fn extract_keys(
            from: &mut BTreeMap<Name, ConstValue>,
            prefix: usize,
            possible_type: Option<&str>,
        ) -> Representation {
            let prefix = format!("__key{}_", prefix);

            if let Some(possible_type) = possible_type {
                match from.get(format!("{}__typename", prefix).as_str()) {
                    Some(ConstValue::String(typename)) if typename == possible_type => {}
                    _ => return Representation::Skip,
                }
            }

            let mut res = BTreeMap::new();
            let mut keys = Vec::new();
            for key in from.keys() {
                if key.as_str().starts_with(&prefix) {
                    keys.push(key.clone());
                }
            }
            for key in keys {
                if let Some(value) = from.remove(&key) {
                    let name = Name::new(&key[prefix.len()..]);
                    res.insert(name, value);
                }
            }
            Representation::Keys(ConstValue::Object(res))
        }

        fn get_representations(
            representations: &mut Vec<Representation>,
            value: &mut ConstValue,
            path: &[PathSegment<'_>],
            prefix: usize,
        ) {
            let segment = match path.get(0) {
                Some(segment) => segment,
                None => return,
            };
            let is_last = path.len() == 1;

            if is_last {
                match value {
                    ConstValue::Object(object) if !segment.is_list => {
                        if let Some(ConstValue::Object(key_object)) = object.get_mut(segment.name) {
                            representations.push(extract_keys(
                                key_object,
                                prefix,
                                segment.possible_type,
                            ));
                        }
                    }
                    ConstValue::Object(object) if segment.is_list => {
                        if let Some(ConstValue::List(array)) = object.get_mut(segment.name) {
                            for element in array {
                                if let ConstValue::Object(element_obj) = element {
                                    representations.push(extract_keys(
                                        element_obj,
                                        prefix,
                                        segment.possible_type,
                                    ));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                match value {
                    ConstValue::Object(object) if !segment.is_list => {
                        if let Some(next_value) = object.get_mut(segment.name) {
                            get_representations(representations, next_value, &path[1..], prefix);
                        }
                    }
                    ConstValue::Object(object) if segment.is_list => {
                        if let Some(ConstValue::List(array)) = object.get_mut(segment.name) {
                            for element in array {
                                get_representations(representations, element, &path[1..], prefix);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        fn flatten_values(
            target: &mut ConstValue,
            path: &[PathSegment<'_>],
            values: &mut impl Iterator<Item = ConstValue>,
            flags: &mut impl Iterator<Item = bool>,
        ) {
            let segment = match path.get(0) {
                Some(segment) => segment,
                None => return,
            };
            let is_last = path.len() == 1;

            if is_last {
                match target {
                    ConstValue::Object(object) if !segment.is_list => {
                        if let Some(target) = object.get_mut(segment.name) {
                            if let Some(true) = flags.next() {
                                if let Some(value) = values.next() {
                                    merge_data(target, value);
                                }
                            }
                        }
                    }
                    ConstValue::Object(object) if segment.is_list => {
                        if let Some(ConstValue::List(array)) = object.get_mut(segment.name) {
                            for element in array {
                                if let Some(true) = flags.next() {
                                    if let Some(value) = values.next() {
                                        merge_data(element, value);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                match target {
                    ConstValue::Object(object) if !segment.is_list => {
                        if let Some(next_value) = object.get_mut(segment.name) {
                            flatten_values(next_value, &path[1..], values, flags);
                        }
                    }
                    ConstValue::Object(object) if segment.is_list => {
                        if let Some(ConstValue::List(array)) = object.get_mut(segment.name) {
                            for element in array {
                                flatten_values(element, &path[1..], values, flags);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let (representations, flags) = {
            let mut representations = Vec::new();
            let mut resp = self.resp.lock();
            get_representations(
                &mut representations,
                &mut resp.data,
                &flatten.path,
                flatten.prefix,
            );
            if representations.is_empty() {
                return;
            }

            let mut flags = Vec::with_capacity(representations.len());
            let mut values = Vec::with_capacity(representations.len());

            for representation in representations {
                match representation {
                    Representation::Keys(value) => {
                        values.push(value);
                        flags.push(true);
                    }
                    Representation::Skip => flags.push(false),
                }
            }

            let mut variables = Variables::default();
            variables.insert(Name::new("representations"), ConstValue::List(values));
            (variables, flags)
        };
        let request = flatten.to_request(representations);

        let tracer = global::tracer("graphql");
        let span = tracer
            .span_builder(&format!("flatten [{}]", flatten.service))
            .with_kind(SpanKind::Server)
            .with_attributes(vec![
                KeyValue::new(KEY_SERVICE, flatten.service.to_string()),
                KeyValue::new(KEY_QUERY, flatten.query.to_string()),
                KeyValue::new(
                    KEY_VARIABLES,
                    serde_json::to_string(&request.variables).unwrap(),
                ),
                KeyValue::new(KEY_PATH, flatten.path.to_string()),
            ])
            .start(&tracer);
        let cx = Context::current_with_span(span);

        async move {
            let res = fetcher.query(flatten.service, request).await;
            let current_resp = &mut self.resp.lock();

            match res {
                Ok(mut resp) => {
                    if resp.errors.is_empty() {
                        add_tracing_spans(&mut resp);
                        if let ConstValue::Object(mut data) = resp.data {
                            if let Some(ConstValue::List(values)) = data.remove("_entities") {
                                flatten_values(
                                    &mut current_resp.data,
                                    &flatten.path,
                                    &mut values.into_iter().fuse(),
                                    &mut flags.into_iter().fuse(),
                                );
                            }
                        }
                    } else {
                        merge_errors(&mut current_resp.errors, resp.errors);
                    }
                }
                Err(err) => {
                    current_resp.errors.push(ServerError {
                        message: err.to_string(),
                        locations: Default::default(),
                    });
                }
            }
        }
        .with_context(cx)
        .await
    }
}

fn merge_data(target: &mut ConstValue, value: ConstValue) {
    match (target, value) {
        (target @ ConstValue::Null, fragment) => *target = fragment,
        (ConstValue::Object(object), ConstValue::Object(fragment_object)) => {
            for (key, value) in fragment_object {
                match object.get_mut(&key) {
                    Some(target) => merge_data(target, value),
                    None => {
                        object.insert(key, value);
                    }
                }
            }
        }
        (ConstValue::List(array), ConstValue::List(fragment_array))
            if array.len() == fragment_array.len() =>
        {
            for (idx, element) in fragment_array.into_iter().enumerate() {
                merge_data(&mut array[idx], element);
            }
        }
        _ => {}
    }
}

fn merge_errors(target: &mut Vec<ServerError>, errors: Vec<ServerError>) {
    for err in errors {
        target.push(ServerError {
            message: err.message,
            locations: Default::default(),
        })
    }
}

fn add_tracing_spans(response: &mut Response) {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TracingResult {
        version: i32,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        execution: TracingExecution,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TracingExecution {
        resolvers: Vec<TracingResolver>,
    }

    #[derive(Default)]
    struct Path {
        path: String,
        parent_end: usize,
    }

    impl Path {
        fn full_path(&self) -> &str {
            &self.path
        }

        fn parent_path(&self) -> &str {
            &self.path[..self.parent_end]
        }
    }

    fn deserialize_path<'de, D>(deserialize: D) -> Result<Path, D::Error>
    where
        D: Deserializer<'de>,
    {
        let segments = Vec::<ConstValue>::deserialize(deserialize)?;

        fn write_path<W: std::fmt::Write>(w: &mut W, value: &ConstValue) {
            match value {
                ConstValue::Number(idx) => {
                    write!(w, "{}", idx).unwrap();
                }
                ConstValue::String(name) => {
                    write!(w, "{}", name).unwrap();
                }
                _ => {}
            }
        }

        match segments.split_last() {
            Some((last, parents)) => {
                let mut full_path = String::new();
                for (idx, p) in parents.iter().enumerate() {
                    if idx > 0 {
                        full_path.push('.');
                    }
                    write_path(&mut full_path, p);
                }
                let parent_end = full_path.len();
                if !full_path.is_empty() {
                    full_path.push('.');
                }
                write_path(&mut full_path, last);
                Ok(Path {
                    path: full_path,
                    parent_end,
                })
            }
            None => Ok(Path::default()),
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TracingResolver {
        #[serde(deserialize_with = "deserialize_path")]
        path: Path,
        field_name: String,
        parent_type: String,
        return_type: String,
        start_offset: i64,
        duration: i64,
    }

    let tracing_result = match response
        .extensions
        .remove("tracing")
        .and_then(|value| value::from_value::<TracingResult>(value).ok())
    {
        Some(tracing_result) => tracing_result,
        None => return,
    };

    if tracing_result.version != 1 {
        return;
    }

    let tracer = global::tracer("graphql");
    let _root = Context::current_with_span(
        tracer
            .span_builder("execute")
            .with_start_time(tracing_result.start_time)
            .with_end_time(tracing_result.end_time)
            .with_kind(SpanKind::Server)
            .start(&tracer),
    )
    .attach();

    let mut resolvers = HashMap::<_, Context>::new();
    for resolver in &tracing_result.execution.resolvers {
        let attributes = vec![
            KeyValue::new(KEY_PARENT_TYPE, resolver.parent_type.clone()),
            KeyValue::new(KEY_RETURN_TYPE, resolver.return_type.clone()),
            KeyValue::new(KEY_FIELD_NAME, resolver.field_name.clone()),
        ];

        match resolvers.get(resolver.path.parent_path()) {
            Some(parent_ctx) => {
                let current_ctx = Context::current_with_span(
                    tracer
                        .span_builder(resolver.path.full_path())
                        .with_parent_context(parent_ctx.clone())
                        .with_start_time(
                            tracing_result.start_time
                                + Duration::nanoseconds(resolver.start_offset),
                        )
                        .with_end_time(
                            tracing_result.start_time
                                + Duration::nanoseconds(resolver.start_offset)
                                + Duration::nanoseconds(resolver.duration),
                        )
                        .with_attributes(attributes)
                        .with_kind(SpanKind::Server)
                        .start(&tracer),
                );
                resolvers.insert(resolver.path.full_path(), current_ctx);
            }
            None => {
                let current_ctx = Context::current_with_span(
                    tracer
                        .span_builder(resolver.path.full_path())
                        .with_start_time(
                            tracing_result.start_time
                                + Duration::nanoseconds(resolver.start_offset),
                        )
                        .with_end_time(
                            tracing_result.start_time
                                + Duration::nanoseconds(resolver.start_offset)
                                + Duration::nanoseconds(resolver.duration),
                        )
                        .with_attributes(attributes)
                        .with_kind(SpanKind::Server)
                        .start(&tracer),
                );
                resolvers.insert(resolver.path.full_path(), current_ctx);
            }
        }
    }
}
