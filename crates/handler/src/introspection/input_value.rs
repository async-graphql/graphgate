use graphgate_planner::IntrospectionSelectionSet;
use graphgate_schema::{ComposedSchema, MetaInputValue};
use value::ConstValue;

use super::{
    r#type::IntrospectionType,
    resolver::{resolve_obj, Resolver},
};

pub struct IntrospectionInputValue<'a>(pub &'a MetaInputValue);

impl<'a> Resolver for IntrospectionInputValue<'a> {
    fn resolve(
        &self,
        selection_set: &IntrospectionSelectionSet,
        schema: &ComposedSchema,
    ) -> ConstValue {
        resolve_obj(selection_set, |name, field| match name {
            "name" => ConstValue::String(self.0.name.to_string()),
            "description" => self
                .0
                .description
                .as_ref()
                .map(|description| ConstValue::String(description.clone()))
                .unwrap_or_default(),
            "type" => {
                IntrospectionType::new(&self.0.ty, schema).resolve(&field.selection_set, schema)
            }
            "defaultValue" => match &self.0.default_value {
                Some(value) => ConstValue::String(value.to_string()),
                None => ConstValue::Null,
            },
            _ => ConstValue::Null,
        })
    }
}
