#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;

use graphgate_schema::{ComposedSchema, KeyFields, MetaField, MetaType, TypeKind, ValueExt};
use indexmap::IndexMap;
use parser::types::{
    BaseType, DocumentOperations, ExecutableDocument, Field, FragmentDefinition,
    OperationDefinition, OperationType, Selection, SelectionSet, Type, VariableDefinition,
};
use parser::Positioned;
use value::{ConstValue, Name, Value, Variables};

use crate::plan::{
    FetchNode, FlattenNode, IntrospectionDirective, IntrospectionField, IntrospectionNode,
    IntrospectionSelectionSet, ParallelNode, PathSegment, PlanNode, ResponsePath, SequenceNode,
};
use crate::types::{
    FetchEntity, FetchEntityGroup, FetchEntityKey, FetchQuery, FieldRef, MutationRootGroup,
    QueryRootGroup, RequiredRef, RootGroup, SelectionRef, SelectionRefSet, VariableDefinitionsRef,
    VariablesRef,
};
use crate::{Response, RootNode, ServerError, SubscribeNode};

#[derive(Debug)]
struct Context<'a> {
    schema: &'a ComposedSchema,
    fragments: &'a HashMap<Name, Positioned<FragmentDefinition>>,
    variables: &'a Variables,
    key_id: usize,
}

/// Query plan generator
pub struct PlanBuilder<'a> {
    schema: &'a ComposedSchema,
    document: ExecutableDocument,
    operation_name: Option<String>,
    variables: Variables,
}

impl<'a> PlanBuilder<'a> {
    pub fn new(schema: &'a ComposedSchema, document: ExecutableDocument) -> Self {
        Self {
            schema,
            document,
            operation_name: None,
            variables: Default::default(),
        }
    }

    pub fn operation_name(mut self, operation: impl Into<String>) -> Self {
        self.operation_name = Some(operation.into());
        self
    }

    pub fn variables(self, variables: Variables) -> Self {
        Self { variables, ..self }
    }

    fn check_rules(&self) -> Result<(), Response> {
        let rule_errors =
            graphgate_validation::check_rules(self.schema, &self.document, &self.variables);
        if !rule_errors.is_empty() {
            return Err(Response {
                data: ConstValue::Null,
                errors: rule_errors
                    .into_iter()
                    .map(|err| ServerError {
                        message: err.message,
                        path: Default::default(),
                        locations: err.locations,
                        extensions: Default::default(),
                    })
                    .collect(),
                extensions: Default::default(),
                headers: Default::default(),
            });
        }
        Ok(())
    }

    fn create_context(&self) -> Context<'_> {
        let fragments = &self.document.fragments;
        Context {
            schema: self.schema,
            fragments,
            variables: &self.variables,
            key_id: 1,
        }
    }

    pub fn plan(&self) -> Result<RootNode, Response> {
        self.check_rules()?;

        let mut ctx = self.create_context();
        let operation_definition = get_operation(&self.document, self.operation_name.as_deref());

        let root_type = match operation_definition.node.ty {
            OperationType::Query => ctx.schema.query_type(),
            OperationType::Mutation => ctx
                .schema
                .mutation_type()
                .expect("The query validator should find this error."),
            OperationType::Subscription => ctx
                .schema
                .subscription_type()
                .expect("The query validator should find this error."),
        };

        if let Some(root_type) = ctx.schema.types.get(root_type) {
            match operation_definition.node.ty {
                OperationType::Query => Ok(RootNode::Query(ctx.build_root_selection_set(
                    QueryRootGroup::default(),
                    operation_definition.node.ty,
                    &operation_definition.node.variable_definitions,
                    root_type,
                    &operation_definition.node.selection_set.node,
                ))),
                OperationType::Mutation => Ok(RootNode::Query(ctx.build_root_selection_set(
                    MutationRootGroup::default(),
                    operation_definition.node.ty,
                    &operation_definition.node.variable_definitions,
                    root_type,
                    &operation_definition.node.selection_set.node,
                ))),
                OperationType::Subscription => Ok(RootNode::Subscribe(ctx.build_subscribe(
                    &operation_definition.node.variable_definitions,
                    root_type,
                    &operation_definition.node.selection_set.node,
                ))),
            }
        } else {
            unreachable!("The query validator should find this error.")
        }
    }
}

impl<'a> Context<'a> {
    fn build_root_selection_set(
        &mut self,
        mut root_group: impl RootGroup<'a>,
        operation_type: OperationType,
        variable_definitions: &'a [Positioned<VariableDefinition>],
        parent_type: &'a MetaType,
        selection_set: &'a SelectionSet,
    ) -> PlanNode<'a> {
        fn build_root_selection_set_rec<'a>(
            ctx: &mut Context<'a>,
            root_group: &mut impl RootGroup<'a>,
            fetch_entity_group: &mut FetchEntityGroup<'a>,
            inspection_selection_set: &mut IntrospectionSelectionSet,
            parent_type: &'a MetaType,
            selection_set: &'a SelectionSet,
        ) {
            for selection in &selection_set.items {
                match &selection.node {
                    Selection::Field(field) => {
                        let field_name = field.node.name.node.as_str();
                        let field_definition = match parent_type.fields.get(field_name) {
                            Some(field_definition) => field_definition,
                            None => continue,
                        };
                        if is_introspection_field(field_name) {
                            ctx.build_introspection_field(inspection_selection_set, &field.node);
                            continue;
                        }

                        if let Some(service) = &field_definition.service {
                            let selection_ref_set = root_group.selection_set_mut(service);
                            let mut path = ResponsePath::default();
                            ctx.build_field(
                                &mut path,
                                selection_ref_set,
                                fetch_entity_group,
                                service,
                                parent_type,
                                &field.node,
                            );
                        }
                    }
                    Selection::FragmentSpread(fragment_spread) => {
                        if let Some(fragment) = ctx
                            .fragments
                            .get(fragment_spread.node.fragment_name.node.as_str())
                        {
                            build_root_selection_set_rec(
                                ctx,
                                root_group,
                                fetch_entity_group,
                                inspection_selection_set,
                                parent_type,
                                &fragment.node.selection_set.node,
                            );
                        }
                    }
                    Selection::InlineFragment(inline_fragment) => {
                        build_root_selection_set_rec(
                            ctx,
                            root_group,
                            fetch_entity_group,
                            inspection_selection_set,
                            parent_type,
                            &inline_fragment.node.selection_set.node,
                        );
                    }
                }
            }
        }

        let mut fetch_entity_group = FetchEntityGroup::default();
        let mut inspection_selection_set = IntrospectionSelectionSet::default();
        build_root_selection_set_rec(
            self,
            &mut root_group,
            &mut fetch_entity_group,
            &mut inspection_selection_set,
            parent_type,
            selection_set,
        );

        let mut nodes = Vec::new();
        if !inspection_selection_set.0.is_empty() {
            nodes.push(PlanNode::Introspection(IntrospectionNode {
                selection_set: inspection_selection_set,
            }));
        }

        let fetch_node = {
            let mut nodes = Vec::new();
            for (service, selection_set) in root_group.into_selection_set() {
                let (variables, variable_definitions) =
                    referenced_variables(&selection_set, self.variables, variable_definitions);
                nodes.push(PlanNode::Fetch(FetchNode {
                    service,
                    variables,
                    query: FetchQuery {
                        entity_type: None,
                        operation_type,
                        variable_definitions,
                        selection_set,
                    },
                }));
            }
            if operation_type == OperationType::Query {
                PlanNode::Parallel(ParallelNode { nodes }).flatten()
            } else {
                PlanNode::Sequence(SequenceNode { nodes }).flatten()
            }
        };
        nodes.push(fetch_node);

        while !fetch_entity_group.is_empty() {
            let mut flatten_nodes = Vec::new();
            let mut next_group = FetchEntityGroup::new();

            for (
                FetchEntityKey {
                    service, mut path, ..
                },
                FetchEntity {
                    parent_type,
                    prefix,
                    fields,
                },
            ) in fetch_entity_group
            {
                let mut selection_ref_set = SelectionRefSet::default();

                for field in fields {
                    self.build_field(
                        &mut path,
                        &mut selection_ref_set,
                        &mut next_group,
                        service,
                        parent_type,
                        field,
                    );
                }

                let (variables, variable_definitions) =
                    referenced_variables(&selection_ref_set, self.variables, variable_definitions);
                flatten_nodes.push(PlanNode::Flatten(FlattenNode {
                    path,
                    prefix,
                    service,
                    variables,
                    query: FetchQuery {
                        entity_type: Some(parent_type.name.as_str()),
                        operation_type: OperationType::Subscription,
                        variable_definitions,
                        selection_set: selection_ref_set,
                    },
                }));
            }

            nodes.push(
                PlanNode::Parallel(ParallelNode {
                    nodes: flatten_nodes,
                })
                .flatten(),
            );
            fetch_entity_group = next_group;
        }

        PlanNode::Sequence(SequenceNode { nodes }).flatten()
    }

    fn build_subscribe(
        &mut self,
        variable_definitions: &'a [Positioned<VariableDefinition>],
        parent_type: &'a MetaType,
        selection_set: &'a SelectionSet,
    ) -> SubscribeNode<'a> {
        let mut root_group = QueryRootGroup::default();
        let mut fetch_entity_group = FetchEntityGroup::default();

        for selection in &selection_set.items {
            if let Selection::Field(field) = &selection.node {
                let field_name = field.node.name.node.as_str();
                let field_definition = match parent_type.fields.get(field_name) {
                    Some(field_definition) => field_definition,
                    None => continue,
                };

                if let Some(service) = &field_definition.service {
                    let selection_ref_set = root_group.selection_set_mut(service);
                    let mut path = ResponsePath::default();
                    self.build_field(
                        &mut path,
                        selection_ref_set,
                        &mut fetch_entity_group,
                        service,
                        parent_type,
                        &field.node,
                    );
                }
            }
        }

        let fetch_nodes = {
            let mut nodes = Vec::new();
            for (service, selection_ref_set) in root_group.into_selection_set() {
                let (variables, variable_definitions) =
                    referenced_variables(&selection_ref_set, self.variables, variable_definitions);
                nodes.push(FetchNode {
                    service,
                    variables,
                    query: FetchQuery {
                        entity_type: None,
                        operation_type: OperationType::Subscription,
                        variable_definitions,
                        selection_set: selection_ref_set,
                    },
                });
            }
            nodes
        };

        let mut query_nodes = Vec::new();
        while !fetch_entity_group.is_empty() {
            let mut flatten_nodes = Vec::new();
            let mut next_group = FetchEntityGroup::new();

            for (
                FetchEntityKey {
                    service, mut path, ..
                },
                FetchEntity {
                    parent_type,
                    prefix,
                    fields,
                },
            ) in fetch_entity_group
            {
                let mut selection_ref_set = SelectionRefSet::default();

                for field in fields {
                    self.build_field(
                        &mut path,
                        &mut selection_ref_set,
                        &mut next_group,
                        service,
                        parent_type,
                        field,
                    );
                }

                let (variables, variable_definitions) =
                    referenced_variables(&selection_ref_set, self.variables, variable_definitions);
                flatten_nodes.push(PlanNode::Flatten(FlattenNode {
                    path,
                    prefix,
                    service,
                    variables,
                    query: FetchQuery {
                        entity_type: Some(parent_type.name.as_str()),
                        operation_type: OperationType::Query,
                        variable_definitions,
                        selection_set: selection_ref_set,
                    },
                }));
            }

            query_nodes.push(
                PlanNode::Parallel(ParallelNode {
                    nodes: flatten_nodes,
                })
                .flatten(),
            );
            fetch_entity_group = next_group;
        }

        SubscribeNode {
            subscribe_nodes: fetch_nodes,
            flatten_node: if query_nodes.is_empty() {
                None
            } else {
                Some(PlanNode::Sequence(SequenceNode { nodes: query_nodes }).flatten())
            },
        }
    }

    fn build_introspection_field(
        &mut self,
        introspection_selection_set: &mut IntrospectionSelectionSet,
        field: &'a Field,
    ) {
        fn build_selection_set<'a>(
            ctx: &mut Context<'a>,
            introspection_selection_set: &mut IntrospectionSelectionSet,
            selection_set: &'a SelectionSet,
        ) {
            for selection in &selection_set.items {
                match &selection.node {
                    Selection::Field(field) => {
                        ctx.build_introspection_field(introspection_selection_set, &field.node);
                    }
                    Selection::FragmentSpread(fragment_spread) => {
                        if let Some(fragment) = ctx
                            .fragments
                            .get(fragment_spread.node.fragment_name.node.as_str())
                        {
                            build_selection_set(
                                ctx,
                                introspection_selection_set,
                                &fragment.node.selection_set.node,
                            );
                        }
                    }
                    Selection::InlineFragment(inline_fragment) => {
                        build_selection_set(
                            ctx,
                            introspection_selection_set,
                            &inline_fragment.node.selection_set.node,
                        );
                    }
                }
            }
        }

        fn convert_arguments(
            ctx: &mut Context,
            arguments: &[(Positioned<Name>, Positioned<Value>)],
        ) -> IndexMap<Name, ConstValue> {
            arguments
                .iter()
                .map(|(name, value)| {
                    (
                        name.node.clone(),
                        value
                            .node
                            .clone()
                            .into_const_with(|name| {
                                Ok::<_, std::convert::Infallible>(
                                    ctx.variables.get(&name).unwrap().clone(),
                                )
                            })
                            .unwrap(),
                    )
                })
                .collect()
        }

        let mut sub_selection_set = IntrospectionSelectionSet::default();
        build_selection_set(self, &mut sub_selection_set, &field.selection_set.node);
        introspection_selection_set.0.push(IntrospectionField {
            name: field.name.node.clone(),
            alias: field.alias.clone().map(|alias| alias.node),
            arguments: convert_arguments(self, &field.arguments),
            directives: field
                .directives
                .iter()
                .map(|directive| IntrospectionDirective {
                    name: directive.node.name.node.clone(),
                    arguments: convert_arguments(self, &directive.node.arguments),
                })
                .collect(),
            selection_set: sub_selection_set,
        });
    }

    fn build_field(
        &mut self,
        path: &mut ResponsePath<'a>,
        selection_ref_set: &mut SelectionRefSet<'a>,
        fetch_entity_group: &mut FetchEntityGroup<'a>,
        current_service: &'a str,
        parent_type: &'a MetaType,
        field: &'a Field,
    ) {
        let field_name = field.name.node.as_str();

        if field_name == "__typename" {
            selection_ref_set
                .0
                .push(SelectionRef::IntrospectionTypename);
            return;
        }

        let field_definition = match parent_type.fields.get(field_name) {
            Some(field_definition) => field_definition,
            None => return,
        };
        let field_type = match self.schema.get_type(&field_definition.ty) {
            Some(field_type) => field_type,
            None => return,
        };

        let service = match field_definition
            .service
            .as_deref()
            .or_else(|| parent_type.owner.as_deref())
        {
            Some(service) => service,
            None => current_service,
        };

        if service != current_service {
            let mut keys = parent_type.keys.get(service).and_then(|x| x.get(0));
            if keys.is_none() {
                if let Some(owner) = &parent_type.owner {
                    keys = parent_type.keys.get(owner).and_then(|x| x.get(0));
                }
            }
            let keys = match keys {
                Some(keys) => keys,
                None => return,
            };
            if !self.field_in_keys(field, keys) {
                self.add_fetch_entity(
                    path,
                    selection_ref_set,
                    fetch_entity_group,
                    parent_type,
                    field,
                    &field_definition,
                    service,
                    keys,
                );
                return;
            }
        }

        path.push(PathSegment {
            name: field.response_key().node.as_str(),
            is_list: is_list(&field_definition.ty),
            possible_type: None,
        });
        let mut sub_selection_set = SelectionRefSet::default();

        if matches!(field_type.kind, TypeKind::Interface | TypeKind::Union) {
            self.build_abstract_selection_set(
                path,
                &mut sub_selection_set,
                fetch_entity_group,
                current_service,
                &field_type,
                &field.selection_set.node,
            );
        } else {
            self.build_selection_set(
                path,
                &mut sub_selection_set,
                fetch_entity_group,
                current_service,
                field_type,
                &field.selection_set.node,
            );
        }

        selection_ref_set.0.push(SelectionRef::FieldRef(FieldRef {
            field,
            selection_set: sub_selection_set,
        }));
        path.pop();
    }

    fn add_fetch_entity(
        &mut self,
        path: &mut ResponsePath<'a>,
        selection_ref_set: &mut SelectionRefSet<'a>,
        fetch_entity_group: &mut FetchEntityGroup<'a>,
        parent_type: &'a MetaType,
        field: &'a Field,
        meta_field: &'a MetaField,
        service: &'a str,
        keys: &'a KeyFields,
    ) {
        let fetch_entity_key = FetchEntityKey {
            service,
            path: path.clone(),
            ty: parent_type.name.as_str(),
        };

        match fetch_entity_group.get_mut(&fetch_entity_key) {
            Some(fetch_entity) => {
                fetch_entity.fields.push(field);
            }
            None => {
                let prefix = self.take_key_prefix();
                selection_ref_set
                    .0
                    .push(SelectionRef::RequiredRef(RequiredRef {
                        prefix,
                        fields: keys,
                        requires: meta_field.requires.as_ref(),
                    }));
                fetch_entity_group.insert(
                    fetch_entity_key,
                    FetchEntity {
                        parent_type,
                        prefix,
                        fields: vec![field],
                    },
                );
            }
        }
    }

    fn build_selection_set(
        &mut self,
        path: &mut ResponsePath<'a>,
        selection_ref_set: &mut SelectionRefSet<'a>,
        fetch_entity_group: &mut FetchEntityGroup<'a>,
        current_service: &'a str,
        parent_type: &'a MetaType,
        selection_set: &'a SelectionSet,
    ) {
        for selection in &selection_set.items {
            match &selection.node {
                Selection::Field(field) => {
                    self.build_field(
                        path,
                        selection_ref_set,
                        fetch_entity_group,
                        current_service,
                        parent_type,
                        &field.node,
                    );
                }
                Selection::FragmentSpread(fragment_spread) => {
                    if let Some(fragment) = self
                        .fragments
                        .get(fragment_spread.node.fragment_name.node.as_str())
                    {
                        self.build_selection_set(
                            path,
                            selection_ref_set,
                            fetch_entity_group,
                            current_service,
                            parent_type,
                            &fragment.node.selection_set.node,
                        );
                    }
                }
                Selection::InlineFragment(inline_fragment) => {
                    self.build_selection_set(
                        path,
                        selection_ref_set,
                        fetch_entity_group,
                        current_service,
                        parent_type,
                        &inline_fragment.node.selection_set.node,
                    );
                }
            }
        }
    }

    fn build_abstract_selection_set(
        &mut self,
        path: &mut ResponsePath<'a>,
        selection_ref_set: &mut SelectionRefSet<'a>,
        fetch_entity_group: &mut FetchEntityGroup<'a>,
        current_service: &'a str,
        parent_type: &'a MetaType,
        selection_set: &'a SelectionSet,
    ) {
        fn build_fields<'a>(
            ctx: &mut Context<'a>,
            path: &mut ResponsePath<'a>,
            selection_ref_set_group: &mut IndexMap<&'a str, SelectionRefSet<'a>>,
            fetch_entity_group: &mut FetchEntityGroup<'a>,
            current_service: &'a str,
            selection_set: &'a SelectionSet,
            possible_type: &'a MetaType,
        ) {
            let current_ty = possible_type.name.as_str();

            for selection in &selection_set.items {
                match &selection.node {
                    Selection::Field(field) => {
                        ctx.build_field(
                            path,
                            selection_ref_set_group.entry(current_ty).or_default(),
                            fetch_entity_group,
                            current_service,
                            possible_type,
                            &field.node,
                        );
                    }
                    Selection::FragmentSpread(fragment_spread) => {
                        if let Some(fragment) =
                            ctx.fragments.get(&fragment_spread.node.fragment_name.node)
                        {
                            if fragment.node.type_condition.node.on.node == current_ty {
                                build_fields(
                                    ctx,
                                    path,
                                    selection_ref_set_group,
                                    fetch_entity_group,
                                    current_service,
                                    &fragment.node.selection_set.node,
                                    possible_type,
                                );
                            } else {
                                let field_type = match ctx
                                    .schema
                                    .types
                                    .get(&fragment.node.type_condition.node.on.node)
                                {
                                    Some(field_type) => field_type,
                                    None => return,
                                };

                                if matches!(field_type.kind, TypeKind::Interface | TypeKind::Union)
                                {
                                    build_fields(
                                        ctx,
                                        path,
                                        selection_ref_set_group,
                                        fetch_entity_group,
                                        current_service,
                                        &fragment.node.selection_set.node,
                                        possible_type,
                                    );
                                }
                            }
                        }
                    }
                    Selection::InlineFragment(inline_fragment) => {
                        match inline_fragment
                            .node
                            .type_condition
                            .as_ref()
                            .map(|node| &node.node)
                        {
                            Some(type_condition) if type_condition.on.node == current_ty => {
                                build_fields(
                                    ctx,
                                    path,
                                    selection_ref_set_group,
                                    fetch_entity_group,
                                    current_service,
                                    &inline_fragment.node.selection_set.node,
                                    possible_type,
                                );
                            }
                            Some(_type_condition) => {
                                // Other type condition
                            }
                            None => {
                                build_fields(
                                    ctx,
                                    path,
                                    selection_ref_set_group,
                                    fetch_entity_group,
                                    current_service,
                                    &inline_fragment.node.selection_set.node,
                                    possible_type,
                                );
                            }
                        }
                    }
                }
            }
        }

        let mut selection_ref_set_group = IndexMap::new();
        for possible_type in &parent_type.possible_types {
            if let Some(ty) = self.schema.types.get(possible_type) {
                path.last_mut().unwrap().possible_type = Some(ty.name.as_str());
                build_fields(
                    self,
                    path,
                    &mut selection_ref_set_group,
                    fetch_entity_group,
                    current_service,
                    selection_set,
                    ty,
                );
                path.last_mut().unwrap().possible_type = None;
            }
        }

        for (ty, sub_selection_ref_set) in selection_ref_set_group
            .into_iter()
            .filter(|(_, selection_ref_set)| !selection_ref_set.0.is_empty())
        {
            selection_ref_set.0.push(SelectionRef::InlineFragment {
                type_condition: Some(ty),
                selection_set: sub_selection_ref_set,
            });
        }
    }

    fn take_key_prefix(&mut self) -> usize {
        let id = self.key_id;
        self.key_id += 1;
        id
    }

    fn field_in_keys(&self, field: &Field, keys: &KeyFields) -> bool {
        fn selection_set_in_keys(
            ctx: &Context<'_>,
            selection_set: &SelectionSet,
            keys: &KeyFields,
        ) -> bool {
            for selection in &selection_set.items {
                match &selection.node {
                    Selection::Field(field) => {
                        if !ctx.field_in_keys(&field.node, keys) {
                            return false;
                        }
                    }
                    Selection::FragmentSpread(fragment_spread) => {
                        if let Some(fragment) = ctx
                            .fragments
                            .get(fragment_spread.node.fragment_name.node.as_str())
                        {
                            if !selection_set_in_keys(ctx, &fragment.node.selection_set.node, keys)
                            {
                                return false;
                            }
                        } else {
                            return false;
                        }
                    }
                    Selection::InlineFragment(inline_fragment) => {
                        if !selection_set_in_keys(
                            ctx,
                            &inline_fragment.node.selection_set.node,
                            keys,
                        ) {
                            return false;
                        }
                    }
                }
            }
            true
        }

        if let Some(children) = keys.get(field.name.node.as_str()) {
            selection_set_in_keys(self, &field.selection_set.node, children)
        } else {
            false
        }
    }
}

#[inline]
fn is_list(ty: &Type) -> bool {
    matches!(ty.base, BaseType::List(_))
}

fn get_operation<'a>(
    document: &'a ExecutableDocument,
    operation_name: Option<&str>,
) -> &'a Positioned<OperationDefinition> {
    let operation = if let Some(operation_name) = operation_name {
        match &document.operations {
            DocumentOperations::Single(_) => None,
            DocumentOperations::Multiple(operations) => operations.get(operation_name),
        }
    } else {
        match &document.operations {
            DocumentOperations::Single(operation) => Some(operation),
            DocumentOperations::Multiple(map) if map.len() == 1 => {
                Some(map.iter().next().unwrap().1)
            }
            DocumentOperations::Multiple(_) => None,
        }
    };
    operation.expect("The query validator should find this error.")
}

fn referenced_variables<'a>(
    selection_set: &SelectionRefSet<'a>,
    variables: &'a Variables,
    variable_definitions: &'a [Positioned<VariableDefinition>],
) -> (VariablesRef<'a>, VariableDefinitionsRef<'a>) {
    fn referenced_variables_rec<'a>(
        selection_set: &SelectionRefSet<'a>,
        variables: &'a Variables,
        variable_definitions: &'a [Positioned<VariableDefinition>],
        variables_ref: &mut VariablesRef<'a>,
        variables_definition_ref: &mut IndexMap<&'a str, &'a VariableDefinition>,
    ) {
        for selection in &selection_set.0 {
            match selection {
                SelectionRef::FieldRef(field) => {
                    for (_, value) in &field.field.arguments {
                        for name in value.node.referenced_variables() {
                            if let Some((value, definition)) = variables.get(name).zip(
                                variable_definitions
                                    .iter()
                                    .find(|d| d.node.name.node.as_str() == name),
                            ) {
                                variables_ref.variables.insert(name, value);
                                variables_definition_ref.insert(name, &definition.node);
                            } else {
                                let definition = variable_definitions
                                    .iter()
                                    .find(|d| d.node.name.node.as_str() == name)
                                    .unwrap();
                                variables_definition_ref.insert(name, &definition.node);
                            }
                        }
                    }

                    for dir in &field.field.directives {
                        for (_, value) in &dir.node.arguments {
                            for name in value.node.referenced_variables() {
                                if let Some((value, definition)) = variables.get(name).zip(
                                    variable_definitions
                                        .iter()
                                        .find(|d| d.node.name.node.as_str() == name),
                                ) {
                                    variables_ref.variables.insert(name, value);
                                    variables_definition_ref.insert(name, &definition.node);
                                } else {
                                    let definition = variable_definitions
                                        .iter()
                                        .find(|d| d.node.name.node.as_str() == name)
                                        .unwrap();
                                    variables_definition_ref.insert(name, &definition.node);
                                }
                            }
                        }
                    }
                    referenced_variables_rec(
                        &field.selection_set,
                        variables,
                        variable_definitions,
                        variables_ref,
                        variables_definition_ref,
                    )
                }

                SelectionRef::InlineFragment { selection_set, .. } => referenced_variables_rec(
                    selection_set,
                    variables,
                    variable_definitions,
                    variables_ref,
                    variables_definition_ref,
                ),
                _ => {}
            }
        }
    }

    let mut variables_ref = VariablesRef::default();
    let mut variable_definition_ref = IndexMap::new();
    referenced_variables_rec(
        selection_set,
        variables,
        variable_definitions,
        &mut variables_ref,
        &mut variable_definition_ref,
    );
    (
        variables_ref,
        VariableDefinitionsRef {
            variables: variable_definition_ref
                .into_iter()
                .map(|(_, value)| value)
                .collect(),
        },
    )
}

#[inline]
fn is_introspection_field(name: &str) -> bool {
    name == "__type" || name == "__schema"
}
