use graphgate_planner::IntrospectionSelectionSet;
use graphgate_schema::{ComposedSchema, MetaType, TypeKind};
use once_cell::sync::Lazy;
use parser::types::{BaseType, Type};
use value::{ConstValue, Name};

use super::{
    enum_value::IntrospectionEnumValue,
    field::IntrospectionField,
    input_value::IntrospectionInputValue,
    resolver::{is_include_deprecated, resolve_obj, Resolver},
};

static SCALAR: Lazy<Name> = Lazy::new(|| Name::new("SCALAR"));
static OBJECT: Lazy<Name> = Lazy::new(|| Name::new("OBJECT"));
static INTERFACE: Lazy<Name> = Lazy::new(|| Name::new("INTERFACE"));
static UNION: Lazy<Name> = Lazy::new(|| Name::new("UNION"));
static ENUM: Lazy<Name> = Lazy::new(|| Name::new("ENUM"));
static INPUT_OBJECT: Lazy<Name> = Lazy::new(|| Name::new("INPUT_OBJECT"));
static NON_NULL: Lazy<Name> = Lazy::new(|| Name::new("NON_NULL"));
static LIST: Lazy<Name> = Lazy::new(|| Name::new("LIST"));

pub enum IntrospectionType<'a> {
    Named(&'a MetaType),
    NonNull(Box<IntrospectionType<'a>>),
    List(Box<IntrospectionType<'a>>),
}

impl<'a> IntrospectionType<'a> {
    pub fn new(ty: &'a Type, schema: &'a ComposedSchema) -> Self {
        if ty.nullable {
            Self::new_base(&ty.base, schema)
        } else {
            IntrospectionType::NonNull(Box::new(Self::new_base(&ty.base, schema)))
        }
    }

    fn new_base(ty: &'a BaseType, schema: &'a ComposedSchema) -> Self {
        match ty {
            BaseType::Named(name) => IntrospectionType::Named(
                schema
                    .types
                    .get(name)
                    .expect("The query validator should find this error."),
            ),
            BaseType::List(ty) => IntrospectionType::List(Box::new(Self::new(ty, schema))),
        }
    }
}

impl<'a> Resolver for IntrospectionType<'a> {
    fn resolve(
        &self,
        selection_set: &IntrospectionSelectionSet,
        schema: &ComposedSchema,
    ) -> ConstValue {
        resolve_obj(selection_set, |name, field| match name {
            "kind" => match self {
                Self::Named(ty) => match ty.kind {
                    TypeKind::Scalar => ConstValue::Enum(SCALAR.clone()),
                    TypeKind::Object => ConstValue::Enum(OBJECT.clone()),
                    TypeKind::Interface => ConstValue::Enum(INTERFACE.clone()),
                    TypeKind::Union => ConstValue::Enum(UNION.clone()),
                    TypeKind::Enum => ConstValue::Enum(ENUM.clone()),
                    TypeKind::InputObject => ConstValue::Enum(INPUT_OBJECT.clone()),
                },
                Self::NonNull(_) => ConstValue::Enum(NON_NULL.clone()),
                Self::List(_) => ConstValue::Enum(LIST.clone()),
            },
            "name" => match self {
                Self::Named(ty) => ConstValue::String(ty.name.to_string()),
                _ => ConstValue::Null,
            },
            "description" => match self {
                Self::Named(ty) => ty
                    .description
                    .as_ref()
                    .map(|description| ConstValue::String(description.clone()))
                    .unwrap_or_default(),
                _ => ConstValue::Null,
            },
            "fields" => match self {
                Self::Named(ty)
                    if ty.kind == TypeKind::Object || ty.kind == TypeKind::Interface =>
                {
                    ConstValue::List(
                        ty.fields
                            .values()
                            .filter(|item| !item.name.starts_with("__"))
                            .filter(|item| {
                                if is_include_deprecated(&field.arguments) {
                                    true
                                } else {
                                    !item.deprecation.is_deprecated()
                                }
                            })
                            .map(|f| IntrospectionField(f).resolve(&field.selection_set, schema))
                            .collect(),
                    )
                }
                _ => ConstValue::Null,
            },
            "interfaces" => match self {
                Self::Named(ty) if ty.kind == TypeKind::Object => ConstValue::List(
                    ty.implements
                        .iter()
                        .map(|name| {
                            IntrospectionType::Named(
                                schema
                                    .types
                                    .get(name)
                                    .expect("The query validator should find this error."),
                            )
                            .resolve(&field.selection_set, schema)
                        })
                        .collect(),
                ),
                _ => ConstValue::Null,
            },
            "possibleTypes" => match self {
                Self::Named(ty) if ty.kind == TypeKind::Interface || ty.kind == TypeKind::Union => {
                    ConstValue::List(
                        ty.possible_types
                            .iter()
                            .map(|name| {
                                IntrospectionType::Named(
                                    schema
                                        .types
                                        .get(name)
                                        .expect("The query validator should find this error."),
                                )
                                .resolve(&field.selection_set, schema)
                            })
                            .collect(),
                    )
                }
                _ => ConstValue::Null,
            },
            "enumValues" => match self {
                Self::Named(ty) if ty.kind == TypeKind::Enum => ConstValue::List(
                    ty.enum_values
                        .values()
                        .filter(|item| {
                            if is_include_deprecated(&field.arguments) {
                                true
                            } else {
                                !item.deprecation.is_deprecated()
                            }
                        })
                        .map(|value| {
                            IntrospectionEnumValue(value).resolve(&field.selection_set, schema)
                        })
                        .collect(),
                ),
                _ => ConstValue::Null,
            },
            "inputFields" => match self {
                Self::Named(ty) if ty.kind == TypeKind::InputObject => ConstValue::List(
                    ty.input_fields
                        .values()
                        .map(|value| {
                            IntrospectionInputValue(value).resolve(&field.selection_set, schema)
                        })
                        .collect(),
                ),
                _ => ConstValue::Null,
            },
            "ofType" => match self {
                Self::Named(_) => ConstValue::Null,
                Self::List(ty) | Self::NonNull(ty) => ty.resolve(&field.selection_set, schema),
            },
            _ => ConstValue::Null,
        })
    }
}
