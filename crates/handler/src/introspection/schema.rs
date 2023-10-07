use graphgate_planner::IntrospectionSelectionSet;
use graphgate_schema::ComposedSchema;
use value::ConstValue;

use super::{
    r#type::IntrospectionType,
    resolver::{resolve_obj, Resolver},
};

pub struct IntrospectionSchema;

impl Resolver for IntrospectionSchema {
    fn resolve(
        &self,
        selection_set: &IntrospectionSelectionSet,
        schema: &ComposedSchema,
    ) -> ConstValue {
        resolve_obj(selection_set, |name, field| match name {
            "types" => ConstValue::List(
                schema
                    .types
                    .values()
                    .filter(|ty| !ty.name.starts_with("__"))
                    .map(|ty| IntrospectionType::Named(ty).resolve(&field.selection_set, schema))
                    .collect(),
            ),
            "queryType" => {
                let query_type = schema
                    .types
                    .get(schema.query_type())
                    .expect("The query validator should find this error.");
                IntrospectionType::Named(query_type).resolve(&field.selection_set, schema)
            }
            "mutationType" => {
                let mutation_type = schema
                    .mutation_type
                    .as_ref()
                    .and_then(|name| schema.types.get(name));
                match mutation_type {
                    Some(ty) => IntrospectionType::Named(ty).resolve(&field.selection_set, schema),
                    None => ConstValue::Null,
                }
            }
            "subscriptionType" => {
                let subscription_type = schema
                    .subscription_type
                    .as_ref()
                    .and_then(|name| schema.types.get(name));
                match subscription_type {
                    Some(ty) => IntrospectionType::Named(ty).resolve(&field.selection_set, schema),
                    None => ConstValue::Null,
                }
            }
            _ => ConstValue::Null,
        })
    }
}
