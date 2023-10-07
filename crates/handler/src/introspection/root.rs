use graphgate_planner::IntrospectionSelectionSet;
use graphgate_schema::ComposedSchema;
use value::ConstValue;

use super::{
    r#type::IntrospectionType,
    resolver::{resolve_obj, Resolver},
    schema::IntrospectionSchema,
};

pub struct IntrospectionRoot;

impl Resolver for IntrospectionRoot {
    fn resolve(
        &self,
        selection_set: &IntrospectionSelectionSet,
        schema: &ComposedSchema,
    ) -> ConstValue {
        resolve_obj(selection_set, |name, field| match name {
            "__schema" => IntrospectionSchema.resolve(&field.selection_set, schema),
            "__type" => {
                if let Some(ConstValue::String(name)) = field.arguments.get("name") {
                    if let Some(ty) = schema.types.get(name.as_str()) {
                        return IntrospectionType::Named(ty).resolve(&field.selection_set, schema);
                    }
                }
                ConstValue::Null
            }
            _ => ConstValue::Null,
        })
    }
}
