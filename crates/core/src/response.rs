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
    pub locations: Vec<Pos>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub data: ConstValue,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub errors: Vec<ServerError>,
}
