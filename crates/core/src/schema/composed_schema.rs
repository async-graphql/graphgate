use std::collections::HashMap;
use std::ops::Deref;

use indexmap::{IndexMap, IndexSet};
use parser::types::{
    self, BaseType, ConstDirective, DirectiveDefinition, DirectiveLocation, DocumentOperations,
    EnumType, InputObjectType, InterfaceType, ObjectType, SchemaDefinition, Selection,
    SelectionSet, ServiceDocument, Type, TypeDefinition, TypeSystemDefinition, UnionType,
};
use parser::{Positioned, Result};
use value::{ConstValue, Name};

use super::type_ext::TypeExt;
use super::CombineError;

#[derive(Debug, Eq, PartialEq)]
pub enum Deprecation {
    NoDeprecated,
    Deprecated { reason: Option<String> },
}

impl Deprecation {
    #[inline]
    pub fn is_deprecated(&self) -> bool {
        matches!(self, Deprecation::Deprecated { .. })
    }

    #[inline]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Deprecation::NoDeprecated => None,
            Deprecation::Deprecated { reason } => reason.as_deref(),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct MetaField {
    pub description: Option<String>,
    pub name: Name,
    pub arguments: IndexMap<Name, MetaInputValue>,
    pub ty: Type,
    pub deprecation: Deprecation,

    pub service: Option<String>,
    pub requires: Option<KeyFields>,
    pub provides: Option<KeyFields>,
}

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum TypeKind {
    Scalar,
    Object,
    Interface,
    Union,
    Enum,
    InputObject,
}

#[derive(Debug, Eq, PartialEq)]
pub struct KeyFields(IndexMap<Name, KeyFields>);

impl Deref for KeyFields {
    type Target = IndexMap<Name, KeyFields>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct MetaEnumValue {
    pub description: Option<String>,
    pub value: Name,
    pub deprecation: Deprecation,
}

#[derive(Debug, Eq, PartialEq)]
pub struct MetaInputValue {
    pub description: Option<String>,
    pub name: Name,
    pub ty: Type,
    pub default_value: Option<ConstValue>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct MetaType {
    pub description: Option<String>,
    pub name: Name,
    pub kind: TypeKind,
    pub owner: Option<String>,
    pub keys: HashMap<String, Vec<KeyFields>>,

    pub is_introspection: bool,
    pub implements: IndexSet<Name>,
    pub fields: IndexMap<Name, MetaField>,
    pub possible_types: IndexSet<Name>,
    pub enum_values: IndexMap<Name, MetaEnumValue>,
    pub input_fields: IndexMap<Name, MetaInputValue>,
}

impl MetaType {
    #[inline]
    pub fn field_by_name(&self, name: &str) -> Option<&MetaField> {
        self.fields.get(name)
    }

    #[inline]
    pub fn is_composite(&self) -> bool {
        matches!(
            self.kind,
            TypeKind::Object | TypeKind::Interface | TypeKind::Union
        )
    }

    #[inline]
    pub fn is_abstract(&self) -> bool {
        matches!(self.kind, TypeKind::Interface | TypeKind::Union)
    }

    #[inline]
    pub fn is_leaf(&self) -> bool {
        matches!(self.kind, TypeKind::Enum | TypeKind::Scalar)
    }

    #[inline]
    pub fn is_input(&self) -> bool {
        matches!(
            self.kind,
            TypeKind::Enum | TypeKind::Scalar | TypeKind::InputObject
        )
    }

    #[inline]
    pub fn is_possible_type(&self, type_name: &str) -> bool {
        match self.kind {
            TypeKind::Interface | TypeKind::Union => self.possible_types.contains(type_name),
            TypeKind::Object => self.name == type_name,
            _ => false,
        }
    }

    pub fn type_overlap(&self, ty: &MetaType) -> bool {
        if std::ptr::eq(self, ty) {
            return true;
        }

        match (self.is_abstract(), ty.is_abstract()) {
            (true, true) => self
                .possible_types
                .iter()
                .any(|type_name| ty.is_possible_type(type_name)),
            (true, false) => self.is_possible_type(&ty.name),
            (false, true) => ty.is_possible_type(&self.name),
            (false, false) => false,
        }
    }
}

#[derive(Debug)]
pub struct MetaDirective {
    pub name: Name,
    pub description: Option<String>,
    pub locations: Vec<DirectiveLocation>,
    pub arguments: IndexMap<Name, MetaInputValue>,
}

#[derive(Debug, Default)]
pub struct ComposedSchema {
    pub(crate) query_type: Option<Name>,
    pub(crate) mutation_type: Option<Name>,
    pub(crate) subscription_type: Option<Name>,
    pub(crate) types: HashMap<Name, MetaType>,
    pub(crate) directives: HashMap<Name, MetaDirective>,
    pub services: IndexMap<String, String>,
}

impl ComposedSchema {
    pub fn parse(document: &str) -> Result<ComposedSchema> {
        Ok(Self::new(parser::parse_schema(document)?))
    }

    pub fn new(document: ServiceDocument) -> ComposedSchema {
        let mut composed_schema = ComposedSchema::default();

        for definition in document.definitions.into_iter() {
            match definition {
                TypeSystemDefinition::Schema(schema) => {
                    convert_schema_definition(&mut composed_schema, schema.node);
                }
                TypeSystemDefinition::Type(type_definition) => {
                    composed_schema.types.insert(
                        type_definition.node.name.node.clone(),
                        convert_type_definition(type_definition.node),
                    );
                }
                TypeSystemDefinition::Directive(_) => {}
            }
        }

        finish_schema(&mut composed_schema);
        composed_schema
    }

    pub fn combine(
        federation_sdl: impl IntoIterator<Item = (String, ServiceDocument)>,
    ) -> ::std::result::Result<Self, CombineError> {
        let mut composed_schema = ComposedSchema::default();
        let root_objects = &["Query", "Mutation", "Subscription"];

        for obj in root_objects {
            let name = Name::new(obj);
            composed_schema.types.insert(
                name.clone(),
                MetaType {
                    description: None,
                    name,
                    kind: TypeKind::Object,
                    owner: None,
                    keys: Default::default(),
                    is_introspection: false,
                    implements: Default::default(),
                    fields: Default::default(),
                    possible_types: Default::default(),
                    enum_values: Default::default(),
                    input_fields: Default::default(),
                },
            );
        }

        composed_schema.query_type = Some(Name::new("Query"));
        composed_schema.mutation_type = Some(Name::new("Mutation"));
        composed_schema.subscription_type = Some(Name::new("Subscription"));

        for (service, doc) in federation_sdl {
            for definition in doc.definitions {
                match definition {
                    TypeSystemDefinition::Type(type_definition) => {
                        if let types::TypeKind::Object(ObjectType { implements, fields }) =
                            type_definition.node.kind
                        {
                            let name = type_definition.node.name.node.clone();
                            let description = type_definition
                                .node
                                .description
                                .map(|description| description.node);
                            let is_extend = type_definition.node.extend;
                            let meta_type = composed_schema
                                .types
                                .entry(name.clone())
                                .or_insert_with(|| MetaType {
                                    description,
                                    name,
                                    kind: TypeKind::Object,
                                    owner: None,
                                    keys: Default::default(),
                                    is_introspection: false,
                                    implements: Default::default(),
                                    fields: Default::default(),
                                    possible_types: Default::default(),
                                    enum_values: Default::default(),
                                    input_fields: Default::default(),
                                });

                            if !is_extend {
                                meta_type.owner = Some(service.clone());
                            };

                            for directive in type_definition.node.directives {
                                if directive.node.name.node.as_str() == "key" {
                                    if let Some(fields) =
                                        get_argument_str(&directive.node.arguments, "fields")
                                    {
                                        if let Some(selection_set) =
                                            parse_fields(fields.node).map(|selection_set| {
                                                Positioned::new(selection_set, directive.pos)
                                            })
                                        {
                                            meta_type
                                                .keys
                                                .entry(service.clone())
                                                .or_default()
                                                .push(convert_key_fields(selection_set.node));
                                        }
                                    }
                                }
                            }

                            meta_type
                                .implements
                                .extend(implements.into_iter().map(|implement| implement.node));

                            for field in fields {
                                if is_extend {
                                    let is_external =
                                        has_directive(&field.node.directives, "external");
                                    if is_external {
                                        continue;
                                    }
                                }

                                if meta_type.fields.contains_key(&field.node.name.node) {
                                    return Err(CombineError::FieldConflicted {
                                        type_name: type_definition.node.name.node.to_string(),
                                        field_name: field.node.name.node.to_string(),
                                    });
                                }
                                let mut meta_field = convert_field_definition(field.node);
                                if is_extend {
                                    meta_field.service = Some(service.clone());
                                }
                                meta_type.fields.insert(meta_field.name.clone(), meta_field);
                            }
                        } else {
                            let meta_type = convert_type_definition(type_definition.node);
                            if let Some(meta_type2) = composed_schema.types.get(&meta_type.name) {
                                if meta_type2 != &meta_type {
                                    return Err(CombineError::DefinitionConflicted {
                                        type_name: meta_type.name.to_string(),
                                    });
                                }
                            }
                            composed_schema
                                .types
                                .insert(meta_type.name.clone(), meta_type);
                        }
                    }
                    TypeSystemDefinition::Schema(_schema_definition) => {
                        return Err(CombineError::SchemaIsNotAllowed)
                    }
                    TypeSystemDefinition::Directive(_directive_definition) => {}
                }
            }
        }

        if let Some(mutation) = composed_schema.types.get("Mutation") {
            if mutation.fields.is_empty() {
                composed_schema.types.remove("Mutation");
                composed_schema.mutation_type = None;
            }
        }

        if let Some(subscription) = composed_schema.types.get("Subscription") {
            if subscription.fields.is_empty() {
                composed_schema.types.remove("Subscription");
                composed_schema.subscription_type = None;
            }
        }

        finish_schema(&mut composed_schema);
        Ok(composed_schema)
    }

    #[inline]
    pub(crate) fn query_type(&self) -> &str {
        self.query_type
            .as_ref()
            .map(|name| name.as_str())
            .unwrap_or("Query")
    }

    #[inline]
    pub(crate) fn mutation_type(&self) -> Option<&str> {
        self.mutation_type.as_ref().map(|name| name.as_str())
    }

    #[inline]
    pub(crate) fn subscription_type(&self) -> Option<&str> {
        self.subscription_type.as_ref().map(|name| name.as_str())
    }

    #[inline]
    pub(crate) fn get_type(&self, ty: &Type) -> Option<&MetaType> {
        let name = match &ty.base {
            BaseType::Named(name) => name.as_str(),
            BaseType::List(ty) => return self.get_type(ty),
        };
        self.types.get(name)
    }

    pub(crate) fn concrete_type_by_name(&self, ty: &Type) -> Option<&MetaType> {
        self.types.get(ty.concrete_typename())
    }
}

fn get_argument<'a>(
    arguments: &'a [(Positioned<Name>, Positioned<ConstValue>)],
    name: &str,
) -> Option<&'a Positioned<ConstValue>> {
    arguments.iter().find_map(|d| {
        if d.0.node.as_str() == name {
            Some(&d.1)
        } else {
            None
        }
    })
}

fn get_argument_str<'a>(
    arguments: &'a [(Positioned<Name>, Positioned<ConstValue>)],
    name: &str,
) -> Option<Positioned<&'a str>> {
    get_argument(arguments, name).and_then(|value| match &value.node {
        ConstValue::String(s) => Some(Positioned::new(s.as_str(), value.pos)),
        _ => None,
    })
}

fn parse_fields(fields: &str) -> Option<SelectionSet> {
    parser::parse_query(format!("{{{}}}", fields))
        .ok()
        .and_then(|document| match document.operations {
            DocumentOperations::Single(op) => Some(op.node.selection_set.node),
            DocumentOperations::Multiple(_) => None,
        })
}

fn convert_schema_definition(
    composed_schema: &mut ComposedSchema,
    schema_definition: SchemaDefinition,
) {
    composed_schema.query_type = schema_definition.query.map(|name| name.node);
    composed_schema.mutation_type = schema_definition.mutation.map(|name| name.node);
    composed_schema.subscription_type = schema_definition.subscription.map(|name| name.node);

    for directive in schema_definition.directives {
        if directive.node.name.node.as_str() == "service" {
            if let Some((name, url)) = get_argument_str(&directive.node.arguments, "name")
                .zip(get_argument_str(&directive.node.arguments, "url"))
            {
                composed_schema
                    .services
                    .insert(name.node.to_string(), url.node.to_string());
            }
        }
    }
}

fn convert_type_definition(definition: TypeDefinition) -> MetaType {
    let mut type_definition = MetaType {
        description: definition.description.map(|description| description.node),
        name: definition.name.node.clone(),
        kind: TypeKind::Scalar,
        owner: None,
        keys: Default::default(),
        is_introspection: false,
        implements: Default::default(),
        fields: Default::default(),
        possible_types: Default::default(),
        enum_values: Default::default(),
        input_fields: Default::default(),
    };

    match definition.kind {
        types::TypeKind::Scalar => type_definition.kind = TypeKind::Scalar,
        types::TypeKind::Object(ObjectType { implements, fields }) => {
            type_definition.kind = TypeKind::Object;
            type_definition.implements = implements
                .into_iter()
                .map(|implement| implement.node)
                .collect();
            type_definition
                .fields
                .extend(fields.into_iter().map(|field| {
                    (
                        field.node.name.node.clone(),
                        convert_field_definition(field.node),
                    )
                }));
        }
        types::TypeKind::Interface(InterfaceType { implements, fields }) => {
            type_definition.kind = TypeKind::Interface;
            type_definition.implements = implements.into_iter().map(|name| name.node).collect();
            type_definition.fields = fields
                .into_iter()
                .map(|field| {
                    (
                        field.node.name.node.clone(),
                        convert_field_definition(field.node),
                    )
                })
                .collect();
        }
        types::TypeKind::Union(UnionType { members }) => {
            type_definition.kind = TypeKind::Union;
            type_definition.possible_types = members.into_iter().map(|name| name.node).collect();
        }
        types::TypeKind::Enum(EnumType { values }) => {
            type_definition.kind = TypeKind::Enum;
            type_definition
                .enum_values
                .extend(values.into_iter().map(|value| {
                    (
                        value.node.value.node.clone(),
                        MetaEnumValue {
                            description: value.node.description.map(|description| description.node),
                            value: value.node.value.node,
                            deprecation: get_deprecated(&value.node.directives),
                        },
                    )
                }));
        }
        types::TypeKind::InputObject(InputObjectType { fields }) => {
            type_definition.kind = TypeKind::InputObject;
            type_definition
                .input_fields
                .extend(fields.into_iter().map(|field| {
                    (
                        field.node.name.node.clone(),
                        convert_input_value_definition(field.node),
                    )
                }));
        }
    }

    for directive in definition.directives {
        match directive.node.name.node.as_str() {
            "owner" => {
                if let Some(service) = get_argument_str(&directive.node.arguments, "service") {
                    type_definition.owner = Some(service.node.to_string());
                }
            }
            "key" => {
                if let Some((fields, service)) =
                    get_argument_str(&directive.node.arguments, "fields")
                        .zip(get_argument_str(&directive.node.arguments, "service"))
                {
                    if let Some(selection_set) = parse_fields(fields.node)
                        .map(|selection_set| Positioned::new(selection_set, directive.pos))
                    {
                        type_definition
                            .keys
                            .entry(service.node.to_string())
                            .or_default()
                            .push(convert_key_fields(selection_set.node));
                    }
                }
            }
            _ => {}
        }
    }

    type_definition
}

fn convert_field_definition(definition: types::FieldDefinition) -> MetaField {
    let mut field_definition = MetaField {
        description: definition.description.map(|description| description.node),
        name: definition.name.node,
        arguments: definition
            .arguments
            .into_iter()
            .map(|arg| {
                (
                    arg.node.name.node.clone(),
                    convert_input_value_definition(arg.node),
                )
            })
            .collect(),
        ty: definition.ty.node,
        deprecation: get_deprecated(&definition.directives),
        service: None,
        requires: None,
        provides: None,
    };

    for directive in definition.directives {
        match directive.node.name.node.as_str() {
            "resolve" => {
                if let Some(service) = get_argument_str(&directive.node.arguments, "service") {
                    field_definition.service = Some(service.node.to_string());
                }
            }
            "requires" => {
                if let Some(fields) = get_argument_str(&directive.node.arguments, "fields") {
                    field_definition.requires = parse_fields(fields.node).map(convert_key_fields);
                }
            }
            "provides" => {
                if let Some(fields) = get_argument_str(&directive.node.arguments, "fields") {
                    field_definition.provides = parse_fields(fields.node).map(convert_key_fields);
                }
            }
            _ => {}
        }
    }

    field_definition
}

fn convert_key_fields(selection_set: SelectionSet) -> KeyFields {
    KeyFields(
        selection_set
            .items
            .into_iter()
            .filter_map(|field| {
                if let Selection::Field(field) = field.node {
                    Some((
                        field.node.name.node,
                        convert_key_fields(field.node.selection_set.node),
                    ))
                } else {
                    None
                }
            })
            .collect(),
    )
}

fn convert_input_value_definition(arg: parser::types::InputValueDefinition) -> MetaInputValue {
    MetaInputValue {
        description: arg.description.map(|description| description.node),
        name: arg.name.node,
        ty: arg.ty.node,
        default_value: arg.default_value.map(|default_value| default_value.node),
    }
}

fn convert_directive_definition(directive_definition: DirectiveDefinition) -> MetaDirective {
    MetaDirective {
        name: directive_definition.name.node,
        description: directive_definition
            .description
            .map(|directive_definition| directive_definition.node),
        locations: directive_definition
            .locations
            .into_iter()
            .map(|location| location.node)
            .collect(),
        arguments: directive_definition
            .arguments
            .into_iter()
            .map(|arg| {
                (
                    arg.node.name.node.clone(),
                    convert_input_value_definition(arg.node),
                )
            })
            .collect(),
    }
}

fn get_deprecated(directives: &[Positioned<ConstDirective>]) -> Deprecation {
    directives
        .iter()
        .find(|directive| directive.node.name.node.as_str() == "deprecated")
        .map(|directive| Deprecation::Deprecated {
            reason: get_argument_str(&directive.node.arguments, "reason")
                .map(|reason| reason.node.to_string()),
        })
        .unwrap_or(Deprecation::NoDeprecated)
}

fn has_directive(directives: &[Positioned<ConstDirective>], name: &str) -> bool {
    directives
        .iter()
        .any(|directive| directive.node.name.node.as_str() == name)
}

fn finish_schema(composed_schema: &mut ComposedSchema) {
    for definition in parser::parse_schema(include_str!("builtin.graphql"))
        .unwrap()
        .definitions
        .into_iter()
    {
        match definition {
            TypeSystemDefinition::Type(type_definition) => {
                let mut type_definition = convert_type_definition(type_definition.node);
                type_definition.is_introspection = true;
                composed_schema
                    .types
                    .insert(type_definition.name.clone(), type_definition);
            }
            TypeSystemDefinition::Directive(directive_definition) => {
                composed_schema.directives.insert(
                    directive_definition.node.name.node.clone(),
                    convert_directive_definition(directive_definition.node),
                );
            }
            TypeSystemDefinition::Schema(_) => {}
        }
    }

    if let Some(query_type) = composed_schema.types.get_mut(
        composed_schema
            .query_type
            .as_ref()
            .map(|name| name.as_str())
            .unwrap_or("Query"),
    ) {
        let name = Name::new("__type");
        query_type.fields.insert(
            name.clone(),
            MetaField {
                description: None,
                name,
                arguments: {
                    let mut arguments = IndexMap::new();
                    let name = Name::new("name");
                    arguments.insert(
                        name.clone(),
                        MetaInputValue {
                            description: None,
                            name,
                            ty: Type::new("String!").unwrap(),
                            default_value: None,
                        },
                    );
                    arguments
                },
                ty: Type::new("__Type").unwrap(),
                deprecation: Deprecation::NoDeprecated,
                service: None,
                requires: None,
                provides: None,
            },
        );

        let name = Name::new("__schema");
        query_type.fields.insert(
            name.clone(),
            MetaField {
                description: None,
                name,
                arguments: Default::default(),
                ty: Type::new("__Schema!").unwrap(),
                deprecation: Deprecation::NoDeprecated,
                service: None,
                requires: None,
                provides: None,
            },
        );
    }

    let mut possible_types: HashMap<Name, IndexSet<Name>> = Default::default();
    for ty in composed_schema.types.values() {
        if ty.kind == TypeKind::Object {
            for implement in &ty.implements {
                possible_types
                    .entry(implement.clone())
                    .or_default()
                    .insert(ty.name.clone());
            }
        }
    }
    for (name, types) in possible_types {
        if let Some(ty) = composed_schema.types.get_mut(&name) {
            ty.possible_types = types;
        }
    }
}
