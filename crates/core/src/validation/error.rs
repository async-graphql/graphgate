use parser::Pos;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct RuleError {
    pub message: String,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub locations: Vec<Pos>,
}
