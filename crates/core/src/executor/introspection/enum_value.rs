use value::ConstValue;

use super::resolver::{resolve_obj, Resolver};
use crate::planner::IntrospectionSelectionSet;
use crate::schema::{ComposedSchema, Deprecation, MetaEnumValue};

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
            "isDeprecated" => ConstValue::Boolean(matches!(
                &self.0.deprecation,
                Deprecation::Deprecated { .. }
            )),
            "deprecationReason" => match &self.0.deprecation {
                Deprecation::NoDeprecated => ConstValue::Null,
                Deprecation::Deprecated { reason } => reason
                    .as_ref()
                    .map(|reason| ConstValue::String(reason.clone()))
                    .unwrap_or_default(),
            },
            _ => ConstValue::Null,
        })
    }
}
