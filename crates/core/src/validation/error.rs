use parser::Pos;

#[derive(Debug)]
pub struct RuleError {
    pub message: String,
    pub locations: Vec<Pos>,
}
