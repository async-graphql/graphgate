use std::collections::HashMap;

use parser::Pos;
use serde::{Deserialize, Serialize};
use value::ConstValue;

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ErrorPath {
    Name(String),
    Index(usize),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerError {
    pub message: String,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub path: Vec<ConstValue>,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub locations: Vec<Pos>,

    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub extensions: HashMap<String, ConstValue>,
}

impl ServerError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            path: Default::default(),
            locations: Default::default(),
            extensions: Default::default(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Response {
    pub data: ConstValue,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub errors: Vec<ServerError>,

    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub extensions: HashMap<String, ConstValue>,

    #[serde(skip_serializing)]
    pub headers: Option<HashMap<String, String>>,
}
