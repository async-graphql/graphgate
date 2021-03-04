use value::ConstValue;

use super::resolver::{resolve_obj, Resolver};
use crate::planner::IntrospectionSelectionSet;
use crate::schema::{ComposedSchema, MetaEnumValue};

pub struct IntrospectionEnumValue<'a>(pub &'a MetaEnumValue);

impl<'a> Resolver for IntrospectionEnumValue<'a> {
    fn resolve(
        &self,
        selection_set: &IntrospectionSelectionSet,
        _schema: &ComposedSchema,
    ) -> ConstValue {
        resolve_obj(selection_set, |name, _field| match name {
            "name" => ConstValue::String(self.0.value.to_string()),
            "description" => self
                .0
                .description
                .as_ref()
                .map(|description| ConstValue::String(description.clone()))
                .unwrap_or_default(),
            "isDeprecated" => ConstValue::Boolean(self.0.deprecation.is_deprecated()),
            "deprecationReason" => self
                .0
                .deprecation
                .reason()
                .map(|reason| ConstValue::String(reason.to_string()))
                .unwrap_or_default(),
            _ => ConstValue::Null,
        })
    }
}
