use std::collections::BTreeMap;

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use graphgate_planner::{
    FetchNode, FlattenNode, IntrospectionNode, ParallelNode, PathSegment, PlanNode, RootNode,
    SequenceNode, SubscribeNode,
};
use graphgate_planner::{Request, Response, ServerError};
use graphgate_schema::ComposedSchema;
use spin::Mutex;
use tokio::sync::mpsc;
use tracing::instrument;
use value::{ConstValue, Name, Variables};

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
                    locations: Default::default(),
                }],
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

    #[instrument(skip(self, fetcher), level = "debug")]
    async fn execute_sequence_node(&self, fetcher: &impl Fetcher, sequence: &SequenceNode<'_>) {
        for node in &sequence.nodes {
            self.execute_node(fetcher, node).await;
        }
    }

    #[instrument(skip(self, fetcher), level = "debug")]
    async fn execute_parallel_node(&self, fetcher: &impl Fetcher, parallel: &ParallelNode<'_>) {
        futures_util::future::join_all(
            parallel
                .nodes
                .iter()
                .map(|node| async move { self.execute_node(fetcher, node).await }),
        )
        .await;
    }

    #[instrument(skip(self), level = "debug")]
    fn execute_introspection_node(&self, introspection: &IntrospectionNode) {
        let value = IntrospectionRoot.resolve(&introspection.selection_set, self.schema);
        let mut current_resp = self.resp.lock();
        merge_data(&mut current_resp.data, value);
    }

    #[instrument(skip(self, fetcher), level = "debug")]
    async fn execute_fetch_node(&self, fetcher: &impl Fetcher, fetch: &FetchNode<'_>) {
        let request = fetch.to_request();
        tracing::debug!(service = fetch.service, request = ?request, "Query");
        let res = fetcher.query(fetch.service, request).await;
        let mut current_resp = self.resp.lock();

        match res {
            Ok(resp) => {
                if resp.errors.is_empty() {
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

    #[instrument(skip(self, fetcher), level = "debug")]
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
        tracing::debug!(service = flatten.service, request = ?request, "Query");
        let res = fetcher.query(flatten.service, request).await;
        let current_resp = &mut self.resp.lock();

        match res {
            Ok(resp) => {
                if resp.errors.is_empty() {
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
