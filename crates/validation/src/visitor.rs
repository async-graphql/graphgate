use std::collections::HashMap;

use graphgate_schema::{ComposedSchema, MetaType, TypeKind};
use parser::{
    types::{
        BaseType, Directive, ExecutableDocument, Field, FragmentDefinition, FragmentSpread,
        InlineFragment, OperationDefinition, OperationType, Selection, SelectionSet, Type,
        TypeCondition, VariableDefinition,
    },
    Pos, Positioned,
};
use value::{Name, Value, Variables};

use crate::RuleError;

pub struct VisitorContext<'a> {
    pub schema: &'a ComposedSchema,
    pub variables: &'a Variables,
    pub errors: Vec<RuleError>,
    type_stack: Vec<Option<&'a MetaType>>,
    input_type: Vec<Option<&'a Type>>,
    fragments: &'a HashMap<Name, Positioned<FragmentDefinition>>,
}

impl<'a> VisitorContext<'a> {
    pub fn new(
        schema: &'a ComposedSchema,
        document: &'a ExecutableDocument,
        variables: &'a Variables,
    ) -> Self {
        Self {
            schema,
            variables,
            errors: Default::default(),
            type_stack: Default::default(),
            input_type: Default::default(),
            fragments: &document.fragments,
        }
    }

    pub fn report_error(&mut self, locations: Vec<Pos>, msg: impl Into<String>) {
        self.errors.push(RuleError {
            locations,
            message: msg.into(),
        });
    }

    pub fn append_errors(&mut self, errors: impl IntoIterator<Item = RuleError>) {
        self.errors.extend(errors);
    }

    pub fn with_type(
        &mut self,
        ty: Option<&'a MetaType>,
        mut f: impl FnMut(&mut VisitorContext<'a>),
    ) {
        self.type_stack.push(ty);
        f(self);
        self.type_stack.pop();
    }

    pub fn with_input_type(
        &mut self,
        ty: Option<&'a Type>,
        mut f: impl FnMut(&mut VisitorContext<'a>),
    ) {
        self.input_type.push(ty);
        f(self);
        self.input_type.pop();
    }

    pub fn parent_type(&self) -> Option<&'a MetaType> {
        if self.type_stack.len() >= 2 {
            self.type_stack
                .get(self.type_stack.len() - 2)
                .copied()
                .flatten()
        } else {
            None
        }
    }

    pub fn current_type(&self) -> Option<&'a MetaType> {
        self.type_stack.last().copied().flatten()
    }

    pub fn is_known_fragment(&self, name: &str) -> bool {
        self.fragments.contains_key(name)
    }

    pub fn fragment(&self, name: &str) -> Option<&'a Positioned<FragmentDefinition>> {
        self.fragments.get(name)
    }
}

#[allow(unused_variables)]
pub trait Visitor<'a> {
    fn enter_document(&mut self, ctx: &mut VisitorContext<'a>, doc: &'a ExecutableDocument) {}
    fn exit_document(&mut self, ctx: &mut VisitorContext<'a>, doc: &'a ExecutableDocument) {}

    fn enter_operation_definition(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        name: Option<&'a Name>,
        operation_definition: &'a Positioned<OperationDefinition>,
    ) {
    }
    fn exit_operation_definition(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        name: Option<&'a Name>,
        operation_definition: &'a Positioned<OperationDefinition>,
    ) {
    }

    fn enter_fragment_definition(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        name: &'a Name,
        fragment_definition: &'a Positioned<FragmentDefinition>,
    ) {
    }
    fn exit_fragment_definition(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        name: &'a Name,
        fragment_definition: &'a Positioned<FragmentDefinition>,
    ) {
    }

    fn enter_variable_definition(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        variable_definition: &'a Positioned<VariableDefinition>,
    ) {
    }
    fn exit_variable_definition(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        variable_definition: &'a Positioned<VariableDefinition>,
    ) {
    }

    fn enter_directive(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        directive: &'a Positioned<Directive>,
    ) {
    }
    fn exit_directive(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        directive: &'a Positioned<Directive>,
    ) {
    }

    fn enter_argument(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        name: &'a Positioned<Name>,
        value: &'a Positioned<Value>,
    ) {
    }
    fn exit_argument(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        name: &'a Positioned<Name>,
        value: &'a Positioned<Value>,
    ) {
    }

    fn enter_selection_set(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        selection_set: &'a Positioned<SelectionSet>,
    ) {
    }
    fn exit_selection_set(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        selection_set: &'a Positioned<SelectionSet>,
    ) {
    }

    fn enter_selection(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        selection: &'a Positioned<Selection>,
    ) {
    }
    fn exit_selection(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        selection: &'a Positioned<Selection>,
    ) {
    }

    fn enter_field(&mut self, ctx: &mut VisitorContext<'a>, field: &'a Positioned<Field>) {}
    fn exit_field(&mut self, ctx: &mut VisitorContext<'a>, field: &'a Positioned<Field>) {}

    fn enter_fragment_spread(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        fragment_spread: &'a Positioned<FragmentSpread>,
    ) {
    }
    fn exit_fragment_spread(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        fragment_spread: &'a Positioned<FragmentSpread>,
    ) {
    }

    fn enter_inline_fragment(
        &mut self,
        _ctx: &mut VisitorContext<'a>,
        _inline_fragment: &'a Positioned<InlineFragment>,
    ) {
    }
    fn exit_inline_fragment(
        &mut self,
        _ctx: &mut VisitorContext<'a>,
        _inline_fragment: &'a Positioned<InlineFragment>,
    ) {
    }

    fn enter_input_value(
        &mut self,
        _ctx: &mut VisitorContext<'a>,
        _pos: Pos,
        _expected_type: &Option<&'a Type>,
        _value: &'a Value,
    ) {
    }
    fn exit_input_value(
        &mut self,
        _ctx: &mut VisitorContext<'a>,
        _pos: Pos,
        _expected_type: &Option<&'a Type>,
        _value: &Value,
    ) {
    }
}

pub struct VisitorNil;

impl VisitorNil {
    pub fn with<V>(self, visitor: V) -> VisitorCons<V, Self> {
        VisitorCons(visitor, self)
    }
}

impl<'a> Visitor<'a> for VisitorNil {}

pub struct VisitorCons<A, B>(A, B);

impl<A, B> VisitorCons<A, B> {
    pub fn with<V>(self, visitor: V) -> VisitorCons<V, Self> {
        VisitorCons(visitor, self)
    }
}

impl<'a, A, B> Visitor<'a> for VisitorCons<A, B>
where
    A: Visitor<'a> + 'a,
    B: Visitor<'a> + 'a,
{
    fn enter_document(&mut self, ctx: &mut VisitorContext<'a>, doc: &'a ExecutableDocument) {
        self.0.enter_document(ctx, doc);
        self.1.enter_document(ctx, doc);
    }

    fn exit_document(&mut self, ctx: &mut VisitorContext<'a>, doc: &'a ExecutableDocument) {
        self.0.exit_document(ctx, doc);
        self.1.exit_document(ctx, doc);
    }

    fn enter_operation_definition(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        name: Option<&'a Name>,
        operation_definition: &'a Positioned<OperationDefinition>,
    ) {
        self.0
            .enter_operation_definition(ctx, name, operation_definition);
        self.1
            .enter_operation_definition(ctx, name, operation_definition);
    }

    fn exit_operation_definition(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        name: Option<&'a Name>,
        operation_definition: &'a Positioned<OperationDefinition>,
    ) {
        self.0
            .exit_operation_definition(ctx, name, operation_definition);
        self.1
            .exit_operation_definition(ctx, name, operation_definition);
    }

    fn enter_fragment_definition(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        name: &'a Name,
        fragment_definition: &'a Positioned<FragmentDefinition>,
    ) {
        self.0
            .enter_fragment_definition(ctx, name, fragment_definition);
        self.1
            .enter_fragment_definition(ctx, name, fragment_definition);
    }

    fn exit_fragment_definition(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        name: &'a Name,
        fragment_definition: &'a Positioned<FragmentDefinition>,
    ) {
        self.0
            .exit_fragment_definition(ctx, name, fragment_definition);
        self.1
            .exit_fragment_definition(ctx, name, fragment_definition);
    }

    fn enter_variable_definition(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        variable_definition: &'a Positioned<VariableDefinition>,
    ) {
        self.0.enter_variable_definition(ctx, variable_definition);
        self.1.enter_variable_definition(ctx, variable_definition);
    }

    fn exit_variable_definition(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        variable_definition: &'a Positioned<VariableDefinition>,
    ) {
        self.0.exit_variable_definition(ctx, variable_definition);
        self.1.exit_variable_definition(ctx, variable_definition);
    }

    fn enter_directive(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        directive: &'a Positioned<Directive>,
    ) {
        self.0.enter_directive(ctx, directive);
        self.1.enter_directive(ctx, directive);
    }

    fn exit_directive(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        directive: &'a Positioned<Directive>,
    ) {
        self.0.exit_directive(ctx, directive);
        self.1.exit_directive(ctx, directive);
    }

    fn enter_argument(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        name: &'a Positioned<Name>,
        value: &'a Positioned<Value>,
    ) {
        self.0.enter_argument(ctx, name, value);
        self.1.enter_argument(ctx, name, value);
    }

    fn exit_argument(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        name: &'a Positioned<Name>,
        value: &'a Positioned<Value>,
    ) {
        self.0.exit_argument(ctx, name, value);
        self.1.exit_argument(ctx, name, value);
    }

    fn enter_selection_set(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        selection_set: &'a Positioned<SelectionSet>,
    ) {
        self.0.enter_selection_set(ctx, selection_set);
        self.1.enter_selection_set(ctx, selection_set);
    }

    fn exit_selection_set(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        selection_set: &'a Positioned<SelectionSet>,
    ) {
        self.0.exit_selection_set(ctx, selection_set);
        self.1.exit_selection_set(ctx, selection_set);
    }

    fn enter_selection(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        selection: &'a Positioned<Selection>,
    ) {
        self.0.enter_selection(ctx, selection);
        self.1.enter_selection(ctx, selection);
    }

    fn exit_selection(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        selection: &'a Positioned<Selection>,
    ) {
        self.0.exit_selection(ctx, selection);
        self.1.exit_selection(ctx, selection);
    }

    fn enter_field(&mut self, ctx: &mut VisitorContext<'a>, field: &'a Positioned<Field>) {
        self.0.enter_field(ctx, field);
        self.1.enter_field(ctx, field);
    }

    fn exit_field(&mut self, ctx: &mut VisitorContext<'a>, field: &'a Positioned<Field>) {
        self.0.exit_field(ctx, field);
        self.1.exit_field(ctx, field);
    }

    fn enter_fragment_spread(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        fragment_spread: &'a Positioned<FragmentSpread>,
    ) {
        self.0.enter_fragment_spread(ctx, fragment_spread);
        self.1.enter_fragment_spread(ctx, fragment_spread);
    }

    fn exit_fragment_spread(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        fragment_spread: &'a Positioned<FragmentSpread>,
    ) {
        self.0.exit_fragment_spread(ctx, fragment_spread);
        self.1.exit_fragment_spread(ctx, fragment_spread);
    }

    fn enter_inline_fragment(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        inline_fragment: &'a Positioned<InlineFragment>,
    ) {
        self.0.enter_inline_fragment(ctx, inline_fragment);
        self.1.enter_inline_fragment(ctx, inline_fragment);
    }

    fn exit_inline_fragment(
        &mut self,
        ctx: &mut VisitorContext<'a>,
        inline_fragment: &'a Positioned<InlineFragment>,
    ) {
        self.0.exit_inline_fragment(ctx, inline_fragment);
        self.1.exit_inline_fragment(ctx, inline_fragment);
    }
}

pub fn visit<'a, V: Visitor<'a>>(
    v: &mut V,
    ctx: &mut VisitorContext<'a>,
    doc: &'a ExecutableDocument,
) {
    v.enter_document(ctx, doc);

    for (name, fragment) in &doc.fragments {
        ctx.with_type(
            ctx.schema
                .types
                .get(fragment.node.type_condition.node.on.node.as_str()),
            |ctx| visit_fragment_definition(v, ctx, name, fragment),
        )
    }

    for (name, operation) in doc.operations.iter() {
        visit_operation_definition(v, ctx, name, operation);
    }

    v.exit_document(ctx, doc);
}

fn visit_operation_definition<'a, V: Visitor<'a>>(
    v: &mut V,
    ctx: &mut VisitorContext<'a>,
    name: Option<&'a Name>,
    operation: &'a Positioned<OperationDefinition>,
) {
    v.enter_operation_definition(ctx, name, operation);
    let root_name = match &operation.node.ty {
        OperationType::Query => Some(ctx.schema.query_type()),
        OperationType::Mutation => ctx.schema.mutation_type(),
        OperationType::Subscription => ctx.schema.subscription_type(),
    };
    if let Some(root_name) = root_name {
        ctx.with_type(Some(&ctx.schema.types[root_name]), |ctx| {
            visit_variable_definitions(v, ctx, &operation.node.variable_definitions);
            visit_directives(v, ctx, &operation.node.directives);
            visit_selection_set(v, ctx, &operation.node.selection_set);
        });
    } else {
        ctx.report_error(
            vec![operation.pos],
            // The only one with an irregular plural, "query", is always present
            format!("Schema is not configured for {}s.", operation.node.ty),
        );
    }
    v.exit_operation_definition(ctx, name, operation);
}

fn visit_selection_set<'a, V: Visitor<'a>>(
    v: &mut V,
    ctx: &mut VisitorContext<'a>,
    selection_set: &'a Positioned<SelectionSet>,
) {
    if !selection_set.node.items.is_empty() {
        v.enter_selection_set(ctx, selection_set);
        for selection in &selection_set.node.items {
            visit_selection(v, ctx, selection);
        }
        v.exit_selection_set(ctx, selection_set);
    }
}

fn visit_selection<'a, V: Visitor<'a>>(
    v: &mut V,
    ctx: &mut VisitorContext<'a>,
    selection: &'a Positioned<Selection>,
) {
    v.enter_selection(ctx, selection);
    match &selection.node {
        Selection::Field(field) => {
            if field.node.name.node != "__typename" {
                ctx.with_type(
                    ctx.current_type()
                        .and_then(|ty| ty.field_by_name(&field.node.name.node))
                        .and_then(|schema_field| {
                            ctx.schema.concrete_type_by_name(&schema_field.ty)
                        }),
                    |ctx| {
                        visit_field(v, ctx, field);
                    },
                );
            }
        }
        Selection::FragmentSpread(fragment_spread) => {
            visit_fragment_spread(v, ctx, fragment_spread)
        }
        Selection::InlineFragment(inline_fragment) => {
            if let Some(TypeCondition { on: name }) = &inline_fragment
                .node
                .type_condition
                .as_ref()
                .map(|c| &c.node)
            {
                ctx.with_type(ctx.schema.types.get(name.node.as_str()), |ctx| {
                    visit_inline_fragment(v, ctx, inline_fragment)
                });
            } else {
                visit_inline_fragment(v, ctx, inline_fragment)
            }
        }
    }
    v.exit_selection(ctx, selection);
}

fn visit_field<'a, V: Visitor<'a>>(
    v: &mut V,
    ctx: &mut VisitorContext<'a>,
    field: &'a Positioned<Field>,
) {
    v.enter_field(ctx, field);

    for (name, value) in &field.node.arguments {
        v.enter_argument(ctx, name, value);
        let expected_ty = ctx
            .parent_type()
            .and_then(|ty| ty.field_by_name(&field.node.name.node))
            .and_then(|schema_field| schema_field.arguments.get(&*name.node))
            .map(|input_value| &input_value.ty);
        ctx.with_input_type(expected_ty, |ctx| {
            visit_input_value(v, ctx, field.pos, expected_ty, &value.node)
        });
        v.exit_argument(ctx, name, value);
    }

    visit_directives(v, ctx, &field.node.directives);
    visit_selection_set(v, ctx, &field.node.selection_set);
    v.exit_field(ctx, field);
}

fn visit_input_value<'a, V: Visitor<'a>>(
    v: &mut V,
    ctx: &mut VisitorContext<'a>,
    pos: Pos,
    expected_ty: Option<&'a Type>,
    value: &'a Value,
) {
    v.enter_input_value(ctx, pos, &expected_ty, value);

    match value {
        Value::List(values) => {
            if let Some(expected_ty) = expected_ty {
                let elem_ty = &expected_ty.base;
                if let BaseType::List(expected_ty) = elem_ty {
                    values
                        .iter()
                        .for_each(|value| visit_input_value(v, ctx, pos, Some(expected_ty), value));
                }
            }
        }
        Value::Object(values) => {
            if let Some(expected_ty) = expected_ty {
                let expected_ty = &expected_ty.base;
                if let BaseType::Named(expected_ty) = expected_ty {
                    if let Some(ty) = ctx.schema.types.get(expected_ty.as_str()) {
                        if ty.kind == TypeKind::InputObject {
                            for (item_key, item_value) in values {
                                if let Some(input_value) = ty.input_fields.get(item_key.as_str()) {
                                    visit_input_value(
                                        v,
                                        ctx,
                                        pos,
                                        Some(&input_value.ty),
                                        item_value,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    v.exit_input_value(ctx, pos, &expected_ty, value);
}

fn visit_variable_definitions<'a, V: Visitor<'a>>(
    v: &mut V,
    ctx: &mut VisitorContext<'a>,
    variable_definitions: &'a [Positioned<VariableDefinition>],
) {
    for d in variable_definitions {
        v.enter_variable_definition(ctx, d);
        v.exit_variable_definition(ctx, d);
    }
}

fn visit_directives<'a, V: Visitor<'a>>(
    v: &mut V,
    ctx: &mut VisitorContext<'a>,
    directives: &'a [Positioned<Directive>],
) {
    for d in directives {
        v.enter_directive(ctx, d);

        let schema_directive = ctx.schema.directives.get(d.node.name.node.as_str());

        for (name, value) in &d.node.arguments {
            v.enter_argument(ctx, name, value);
            let expected_ty = schema_directive
                .and_then(|schema_directive| schema_directive.arguments.get(&*name.node))
                .map(|input_ty| &input_ty.ty);
            ctx.with_input_type(expected_ty, |ctx| {
                visit_input_value(v, ctx, d.pos, expected_ty, &value.node)
            });
            v.exit_argument(ctx, name, value);
        }

        v.exit_directive(ctx, d);
    }
}

fn visit_fragment_definition<'a, V: Visitor<'a>>(
    v: &mut V,
    ctx: &mut VisitorContext<'a>,
    name: &'a Name,
    fragment: &'a Positioned<FragmentDefinition>,
) {
    v.enter_fragment_definition(ctx, name, fragment);
    visit_directives(v, ctx, &fragment.node.directives);
    visit_selection_set(v, ctx, &fragment.node.selection_set);
    v.exit_fragment_definition(ctx, name, fragment);
}

fn visit_fragment_spread<'a, V: Visitor<'a>>(
    v: &mut V,
    ctx: &mut VisitorContext<'a>,
    fragment_spread: &'a Positioned<FragmentSpread>,
) {
    v.enter_fragment_spread(ctx, fragment_spread);
    visit_directives(v, ctx, &fragment_spread.node.directives);
    v.exit_fragment_spread(ctx, fragment_spread);
}

fn visit_inline_fragment<'a, V: Visitor<'a>>(
    v: &mut V,
    ctx: &mut VisitorContext<'a>,
    inline_fragment: &'a Positioned<InlineFragment>,
) {
    v.enter_inline_fragment(ctx, inline_fragment);
    visit_directives(v, ctx, &inline_fragment.node.directives);
    visit_selection_set(v, ctx, &inline_fragment.node.selection_set);
    v.exit_inline_fragment(ctx, inline_fragment);
}
