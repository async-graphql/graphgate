use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use graphgate_planner::{
    FetchNode, FlattenNode, IntrospectionNode, ParallelNode, PathSegment, PlanNode, ResponsePath,
    RootNode, SequenceNode, SubscribeNode,
};
use graphgate_planner::{Request, Response, ServerError};
use graphgate_schema::ComposedSchema;
use indexmap::IndexMap;
use opentelemetry::trace::{FutureExt, TraceContextExt, Tracer};
use opentelemetry::{global, Context};
use serde::{Deserialize, Deserializer};
use tokio::sync::{mpsc, Mutex};
use value::{ConstValue, Name, Variables};

use crate::constants::*;
use crate::fetcher::{Fetcher, WebSocketFetcher};
use crate::introspection::{IntrospectionRoot, Resolver};
use crate::websocket::WebSocketController;

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
        match node {
            RootNode::Query(node) => {
                self.execute_node(fetcher, node).await;
                self.resp.into_inner()
            }
            RootNode::Subscribe(_) => Response {
                data: ConstValue::Null,
                errors: vec![ServerError {
                    message: "Not supported".to_string(),
                    path: Default::default(),
                    locations: Default::default(),
                    extensions: Default::default(),
                }],
                extensions: Default::default(),
                headers: Default::default(),
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
                let tracer = global::tracer("graphql");
                let span = tracer.start("subscribe");
                let cx = Context::current_with_span(span);

                let res = {
                    let ws_controller = ws_controller.clone();
                    async move {
                        let (tx, rx) = mpsc::unbounded_channel();

                        futures_util::future::try_join_all(subscribe_nodes.iter().map(|node| {
                            let tracer = global::tracer("graphql");
                            let attributes = vec![
                                KEY_SERVICE.string(node.service.to_string()),
                                KEY_QUERY.string(node.query.to_string()),
                                KEY_VARIABLES
                                    .string(serde_json::to_string(&node.variables).unwrap()),
                            ];
                            let span = tracer
                                .span_builder(&format!("subscribe [{}]", node.service))
                                .with_attributes(attributes)
                                .start(&tracer);
                            let cx = Context::current_with_span(span);
                            ws_controller
                                .subscribe(
                                    id,
                                    node.service,
                                    Request::new(node.query.to_string())
                                        .variables(node.variables.to_variables()),
                                    tx.clone(),
                                )
                                .with_context(cx)
                        }))
                        .await
                        .map(move |_| rx)
                        .map_err(|err| {
                            Context::current().span().add_event(
                                "Failed to subscribe".to_string(),
                                vec![KEY_ERROR.string(err.to_string())],
                            );
                            Response {
                                data: ConstValue::Null,
                                errors: vec![ServerError {
                                    message: err.to_string(),
                                    path: Default::default(),
                                    locations: Default::default(),
                                    extensions: Default::default(),
                                }],
                                extensions: Default::default(),
                                headers: Default::default(),
                            }
                        })
                    }
                    .with_context(cx.clone())
                    .await
                };

                match res {
                    Ok(mut stream) => Box::pin(async_stream::stream! {
                        while let Some(response) = stream.recv().await {
                            if let Some(flatten_node) = flatten_node {
                                *self.resp.lock().await = response;

                                let cx = Context::current_with_span(tracer.span_builder("push").start(&tracer));
                                self.execute_node(&fetcher, flatten_node).with_context(cx).await;

                                yield std::mem::take(&mut *self.resp.lock().await);
                            } else {
                                yield response;
                            }
                        }
                    }.with_context(cx)),
                    Err(response) => {
                        ws_controller.stop(id).await;
                        Box::pin(futures_util::stream::once(async move { response }).boxed())
                    }
                }
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
                    let tracer = global::tracer("graphql");
                    self.execute_introspection_node(introspection)
                        .with_context(Context::current_with_span(tracer.start("introspection")))
                        .await
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

    async fn execute_introspection_node(&self, introspection: &IntrospectionNode) {
        let value = IntrospectionRoot.resolve(&introspection.selection_set, self.schema);
        let mut current_resp = self.resp.lock().await;
        merge_data(&mut current_resp.data, value);
    }

    async fn execute_fetch_node(&self, fetcher: &impl Fetcher, fetch: &FetchNode<'_>) {
        let request = fetch.to_request();

        let tracer = global::tracer("graphql");
        let span = tracer
            .span_builder(&format!("fetch [{}]", fetch.service))
            .with_attributes(vec![
                KEY_SERVICE.string(fetch.service.to_string()),
                KEY_QUERY.string(fetch.query.to_string()),
                KEY_VARIABLES.string(serde_json::to_string(&request.variables).unwrap()),
            ])
            .start(&tracer);
        let cx = Context::current_with_span(span);

        async move {
            let res = fetcher.query(fetch.service, request).await;
            let mut current_resp = self.resp.lock().await;

            match res {
                Ok(mut resp) => {
                    if resp.errors.is_empty() {
                        add_tracing_spans(&mut resp);
                        current_resp.headers = resp.headers;
                        merge_data(&mut current_resp.data, resp.data);
                    } else {
                        rewrite_errors(None, &mut current_resp.errors, resp.errors);
                    }
                }
                Err(err) => current_resp.errors.push(ServerError {
                    message: err.to_string(),
                    path: Default::default(),
                    locations: Default::default(),
                    extensions: Default::default(),
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
            from: &mut IndexMap<Name, ConstValue>,
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

            let mut res = IndexMap::new();
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
                        } else {
                            representations.push(Representation::Skip);
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
                                } else {
                                    representations.push(Representation::Skip);
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
                        } else {
                            representations.push(Representation::Skip);
                        }
                    }
                    ConstValue::Object(object) if segment.is_list => {
                        if let Some(ConstValue::List(array)) = object.get_mut(segment.name) {
                            for element in array {
                                get_representations(representations, element, &path[1..], prefix);
                            }
                        } else {
                            representations.push(Representation::Skip);
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
            let mut resp = self.resp.lock().await;
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
            .with_attributes(vec![
                KEY_SERVICE.string(flatten.service.to_string()),
                KEY_QUERY.string(flatten.query.to_string()),
                KEY_VARIABLES.string(serde_json::to_string(&request.variables).unwrap()),
                KEY_PATH.string(flatten.path.to_string()),
            ])
            .start(&tracer);
        let cx = Context::current_with_span(span);

        async move {
            let res = fetcher.query(flatten.service, request).await;
            let current_resp = &mut self.resp.lock().await;

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
                        rewrite_errors(Some(&flatten.path), &mut current_resp.errors, resp.errors);
                    }
                }
                Err(err) => {
                    current_resp.errors.push(ServerError {
                        message: err.to_string(),
                        path: Default::default(),
                        locations: Default::default(),
                        extensions: Default::default(),
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

fn rewrite_errors(
    prefix_path: Option<&ResponsePath<'_>>,
    target: &mut Vec<ServerError>,
    errors: Vec<ServerError>,
) {
    for mut err in errors {
        let mut path = Vec::new();

        if let Some(prefix_path) = prefix_path {
            for segment in prefix_path.iter() {
                path.push(ConstValue::String(segment.name.to_string()));
                if segment.is_list {
                    path.push(ConstValue::Number(0.into()));
                }
            }
        }

        if matches!(err.path.first(), Some(ConstValue::String(s)) if s=="_entities") {
            path.extend(err.path.drain(1..));
        }

        for subpath in err.path.iter() {
            match subpath {
                ConstValue::String(x) => path.push(ConstValue::String(x.to_string())),
                _ => {}
            }
        }

        target.push(ServerError {
            message: err.message,
            path,
            locations: err.locations,
            extensions: err.extensions,
        })
    }
}

fn add_tracing_spans(response: &mut Response) {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TracingResult {
        version: i32,
        start_time: DateTime<Utc>,
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

    let mut resolvers = HashMap::<_, Context>::new();
    for resolver in &tracing_result.execution.resolvers {
        let attributes = vec![
            KEY_PARENT_TYPE.string(resolver.parent_type.clone()),
            KEY_RETURN_TYPE.string(resolver.return_type.clone()),
            KEY_FIELD_NAME.string(resolver.field_name.clone()),
        ];

        let mut span_builder = tracer
            .span_builder(resolver.path.full_path())
            .with_start_time(
                tracing_result.start_time + Duration::nanoseconds(resolver.start_offset),
            )
            .with_end_time(
                tracing_result.start_time
                    + Duration::nanoseconds(resolver.start_offset)
                    + Duration::nanoseconds(resolver.duration),
            )
            .with_attributes(attributes);

        if let Some(parent_cx) = resolvers.get(resolver.path.parent_path()) {
            span_builder = span_builder.with_parent_context(parent_cx.clone());
        }

        resolvers.insert(
            resolver.path.full_path(),
            Context::current_with_span(span_builder.start(&tracer)),
        );
    }
}
