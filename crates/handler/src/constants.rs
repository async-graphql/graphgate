use opentelemetry::Key;

pub const KEY_SERVICE: Key = Key::from_static_str("graphgate.service");
pub const KEY_QUERY: Key = Key::from_static_str("graphgate.query");
pub const KEY_PATH: Key = Key::from_static_str("graphgate.path");
pub const KEY_PARENT_TYPE: Key = Key::from_static_str("graphgate.parentType");
pub const KEY_RETURN_TYPE: Key = Key::from_static_str("graphgate.returnType");
pub const KEY_FIELD_NAME: Key = Key::from_static_str("graphgate.fieldName");
pub const KEY_VARIABLES: Key = Key::from_static_str("graphgate.variables");
pub const KEY_ERROR: Key = Key::from_static_str("graphgate.error");
