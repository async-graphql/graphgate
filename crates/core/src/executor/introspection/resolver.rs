use std::collections::BTreeMap;

use indexmap::IndexMap;
use value::{ConstValue, Name};

use crate::planner::{IntrospectionDirective, IntrospectionField, IntrospectionSelectionSet};
use crate::ComposedSchema;

pub trait Resolver {
    fn resolve(
        &self,
        selection_set: &IntrospectionSelectionSet,
        schema: &ComposedSchema,
    ) -> ConstValue;
}

pub fn resolve_obj(
    selection_set: &IntrospectionSelectionSet,
    resolve_fn: impl Fn(&str, &IntrospectionField) -> ConstValue,
) -> ConstValue {
    let mut obj = BTreeMap::new();
    for field in &selection_set.0 {
        if is_skip(&field.directives) {
            continue;
        }
        let key = field
            .alias
            .as_ref()
            .cloned()
            .unwrap_or_else(|| field.name.clone());
        obj.insert(key, resolve_fn(field.name.as_str(), field));
    }
    ConstValue::Object(obj)
}

fn is_skip(directives: &[IntrospectionDirective]) -> bool {
    for directive in directives {
        let include = match &*directive.name.as_str() {
            "skip" => false,
            "include" => true,
            _ => continue,
        };

        let condition_input = directive.arguments.get("if").unwrap();
        let value = match condition_input {
            ConstValue::Boolean(value) => *value,
            _ => false,
        };
        return include != value;
    }
    false
}

pub fn is_include_deprecated(arguments: &IndexMap<Name, ConstValue>) -> bool {
    if let Some(ConstValue::Boolean(value)) = arguments.get("includeDeprecated") {
        *value
    } else {
        false
    }
}
