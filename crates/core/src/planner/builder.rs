#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;

use indexmap::IndexMap;
use parser::types::{
    BaseType, DocumentOperations, ExecutableDocument, Field, FragmentDefinition, InlineFragment,
    OperationDefinition, OperationType, Selection, SelectionSet, Type,
};
use parser::Positioned;
use value::{ConstValue, Name, Value, Variables};

use super::plan::{
    FetchNode, FlattenNode, IntrospectionDirective, IntrospectionField, IntrospectionNode,
    IntrospectionSelectionSet, ParallelNode, PathSegment, PlanNode, ResponsePath, SequenceNode,
};
use super::types::{
    FetchEntity, FetchEntityGroup, FetchEntityKey, FieldRef, RequiredRef, RootGroup, SelectionRef,
    SelectionRefSet,
};
use crate::schema::{ComposedSchema, KeyFields, MetaField, MetaType, TypeKind};
use crate::validation::check_rules;
use crate::{Response, ServerError};

struct Context<'a> {
    schema: &'a ComposedSchema,
    fragments: &'a HashMap<Name, Positioned<FragmentDefinition>>,
    variables: &'a Variables,
    key_id: usize,
}

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

    pub fn plan(&self) -> Result<PlanNode, Response> {
        let rule_errors = check_rules(self.schema, &self.document, &self.variables);
        if !rule_errors.is_empty() {
            return Err(Response {
                data: ConstValue::Null,
                errors: rule_errors
                    .into_iter()
                    .map(|err| ServerError {
                        message: err.message,
                        locations: err.locations,
                    })
                    .collect(),
            });
        }

        let fragments = &self.document.fragments;
        let operation_definition = get_operation(&self.document, self.operation_name.as_deref());
        let mut ctx = Context {
            schema: self.schema,
            fragments,
            variables: &self.variables,
            key_id: 1,
        };

        let root_type = match operation_definition.node.ty {
            OperationType::Query => ctx.schema.query_type(),
            OperationType::Mutation => match ctx.schema.mutation_type() {
                Some(mutation_type) => mutation_type,
                None => unreachable!("The query validator should find this error."),
            },
            OperationType::Subscription => {
                unreachable!("The query validator should find this error.")
            }
        };

        if let Some(root_type) = ctx.schema.types.get(root_type) {
            Ok(ctx
                .build_root_selection_set(root_type, &operation_definition.node.selection_set.node))
        } else {
            unreachable!("The query validator should find this error.")
        }
    }
}

impl<'a> Context<'a> {
    fn build_root_selection_set(
        &mut self,
        parent_type: &'a MetaType,
        selection_set: &'a SelectionSet,
    ) -> PlanNode<'a> {
        fn build_root_selection_set_rec<'a>(
            ctx: &mut Context<'a>,
            root_group: &mut RootGroup<'a>,
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
                        let field_type = match ctx.schema.get_type(&field_definition.ty) {
                            Some(field_type) => field_type,
                            None => continue,
                        };
                        if field_type.is_introspection {
                            ctx.build_introspection_field(inspection_selection_set, &field.node);
                            continue;
                        }

                        if let Some(service) = &field_definition.service {
                            let selection_ref_set = root_group.entry(service).or_default();
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

        let mut root_group = RootGroup::default();
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
            for (service, selection_set) in root_group {
                nodes.push(PlanNode::Fetch(FetchNode {
                    service,
                    query: selection_set.to_query(self.variables),
                }));
            }
            PlanNode::Parallel(ParallelNode { nodes }).flatten()
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

                let query = format!(
                    "query($representations:[_Any!]!) {{ _entities(representations:$representations) {{ ... on {} {} }} }}",
                    parent_type.name,
                    selection_ref_set.to_query(self.variables)
                );
                flatten_nodes.push(PlanNode::Flatten(FlattenNode {
                    path,
                    prefix,
                    service,
                    parent_type: parent_type.name.as_str(),
                    query,
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

        if field_type.kind == TypeKind::Interface {
            path.push(PathSegment {
                name: field.response_key().node.as_str(),
                is_list: is_list(&field_definition.ty),
                possible_type: None,
            });
            self.build_interface_field(
                path,
                selection_ref_set,
                fetch_entity_group,
                current_service,
                &field,
                &field_type,
            );
            path.pop();
            return;
        }

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

        let mut sub_selection_set = SelectionRefSet::default();
        path.push(PathSegment {
            name: field.response_key().node.as_str(),
            is_list: is_list(&field_definition.ty),
            possible_type: None,
        });

        self.build_selection_set(
            path,
            &mut sub_selection_set,
            fetch_entity_group,
            current_service,
            field_type,
            &field.selection_set.node,
        );
        path.pop();
        selection_ref_set.0.push(SelectionRef::FieldRef(FieldRef {
            field,
            selection_set: sub_selection_set,
        }));
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
                selection_ref_set
                    .0
                    .push(SelectionRef::RequiredRef(RequiredRef {
                        prefix: fetch_entity.prefix,
                        fields: keys,
                        requires: meta_field.requires.as_ref(),
                    }));
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

    fn build_interface_field(
        &mut self,
        path: &mut ResponsePath<'a>,
        selection_ref_set: &mut SelectionRefSet<'a>,
        fetch_entity_group: &mut FetchEntityGroup<'a>,
        current_service: &'a str,
        field: &'a Field,
        field_type: &'a MetaType,
    ) {
        fn build_fields_rec<'a>(
            ctx: &mut Context<'a>,
            path: &mut ResponsePath<'a>,
            selection_ref_set: &mut SelectionRefSet<'a>,
            fetch_entity_group: &mut FetchEntityGroup<'a>,
            current_service: &'a str,
            selection_set: &'a SelectionSet,
            possible_type: &'a MetaType,
        ) {
            for selection in &selection_set.items {
                match &selection.node {
                    Selection::Field(field) => {
                        ctx.build_field(
                            path,
                            selection_ref_set,
                            fetch_entity_group,
                            current_service,
                            possible_type,
                            &field.node,
                        );
                    }
                    Selection::InlineFragment(inline_fragment)
                        if inline_fragment.node.type_condition.is_none() =>
                    {
                        build_fields_rec(
                            ctx,
                            path,
                            selection_ref_set,
                            fetch_entity_group,
                            current_service,
                            &inline_fragment.node.selection_set.node,
                            possible_type,
                        );
                    }
                    _ => {}
                }
            }
        }

        fn build_fields<'a>(
            ctx: &mut Context<'a>,
            path: &mut ResponsePath<'a>,
            selection_ref_set: &mut SelectionRefSet<'a>,
            fetch_entity_group: &mut FetchEntityGroup<'a>,
            current_service: &'a str,
            selection_set: &'a SelectionSet,
            possible_type: &'a MetaType,
        ) {
            let mut sub_selection_set = SelectionRefSet::default();
            build_fields_rec(
                ctx,
                path,
                &mut sub_selection_set,
                fetch_entity_group,
                current_service,
                &selection_set,
                possible_type,
            );
            if !sub_selection_set.0.is_empty() {
                selection_ref_set.0.push(SelectionRef::InlineFragment {
                    type_condition: Some(possible_type.name.as_str()),
                    selection_set: sub_selection_set,
                });
            }
        }

        fn build_fragment<'a>(
            ctx: &mut Context<'a>,
            path: &mut ResponsePath<'a>,
            selection_ref_set: &mut SelectionRefSet<'a>,
            fetch_entity_group: &mut FetchEntityGroup<'a>,
            current_service: &'a str,
            type_condition: &'a str,
            selection_set: &'a SelectionSet,
            possible_type: &'a MetaType,
        ) {
            let mut sub_selection_set = SelectionRefSet::default();
            path.last_mut().unwrap().possible_type = Some(possible_type.name.as_str());
            build_fields_rec(
                ctx,
                path,
                &mut sub_selection_set,
                fetch_entity_group,
                current_service,
                selection_set,
                possible_type,
            );
            path.last_mut().unwrap().possible_type = None;
            selection_ref_set.0.push(SelectionRef::InlineFragment {
                type_condition: Some(type_condition),
                selection_set: sub_selection_set,
            });
        }

        fn build_type_condition_fragments<'a>(
            ctx: &mut Context<'a>,
            path: &mut ResponsePath<'a>,
            selection_ref_set: &mut SelectionRefSet<'a>,
            fetch_entity_group: &mut FetchEntityGroup<'a>,
            current_service: &'a str,
            selection_set: &'a SelectionSet,
        ) {
            for selection in &selection_set.items {
                match &selection.node {
                    Selection::FragmentSpread(fragment_spread) => {
                        if let Some(fragment) = ctx
                            .fragments
                            .get(fragment_spread.node.fragment_name.node.as_str())
                        {
                            let type_condition = fragment.node.type_condition.node.on.node.as_str();
                            if let Some(ty) = ctx.schema.types.get(type_condition) {
                                build_fragment(
                                    ctx,
                                    path,
                                    selection_ref_set,
                                    fetch_entity_group,
                                    current_service,
                                    type_condition,
                                    &fragment.node.selection_set.node,
                                    ty,
                                );
                            }
                        }
                    }
                    Selection::InlineFragment(Positioned {
                        node:
                            InlineFragment {
                                type_condition:
                                    Some(Positioned {
                                        node: type_condition,
                                        ..
                                    }),
                                selection_set,
                                ..
                            },
                        ..
                    }) => {
                        if let Some(ty) = ctx.schema.types.get(type_condition.on.node.as_str()) {
                            build_fragment(
                                ctx,
                                path,
                                selection_ref_set,
                                fetch_entity_group,
                                current_service,
                                type_condition.on.node.as_str(),
                                &selection_set.node,
                                ty,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }

        for possible_type in &field_type.possible_types {
            if let Some(ty) = self.schema.types.get(possible_type) {
                path.last_mut().unwrap().possible_type = Some(ty.name.as_str());
                build_fields(
                    self,
                    path,
                    selection_ref_set,
                    fetch_entity_group,
                    current_service,
                    &field.selection_set.node,
                    ty,
                );
                path.last_mut().unwrap().possible_type = None;
            }
        }

        build_type_condition_fragments(
            self,
            path,
            selection_ref_set,
            fetch_entity_group,
            current_service,
            &field.selection_set.node,
        );
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
