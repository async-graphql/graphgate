use once_cell::sync::Lazy;
use parser::types::ExecutableDocument;
use value::Variables;

use super::visitor::{visit, Visitor, VisitorContext};
use super::RuleError;
use crate::ComposedSchema;

static SCHEMA: Lazy<ComposedSchema> =
    Lazy::new(|| ComposedSchema::parse(include_str!("test_harness.graphql")).unwrap());

pub fn validate<'a, V, F>(
    doc: &'a ExecutableDocument,
    variables: &'a Variables,
    factory: F,
) -> Result<(), Vec<RuleError>>
where
    V: Visitor<'a> + 'a,
    F: Fn() -> V,
{
    let mut ctx = VisitorContext::new(&*SCHEMA, doc, variables);
    let mut visitor = factory();
    visit(&mut visitor, &mut ctx, doc);
    if ctx.errors.is_empty() {
        Ok(())
    } else {
        Err(ctx.errors)
    }
}

pub fn expect_passes_rule_<'a, V, F>(
    doc: &'a ExecutableDocument,
    variables: &'a Variables,
    factory: F,
) where
    V: Visitor<'a> + 'a,
    F: Fn() -> V,
{
    if let Err(errors) = validate(doc, variables, factory) {
        for err in errors {
            if let Some(position) = err.locations.first() {
                print!("[{}:{}] ", position.line, position.column);
            }
            println!("{}", err.message);
        }
        panic!("Expected rule to pass, but errors found");
    }
}

macro_rules! expect_passes_rule {
    ($factory:expr, $query_source:literal $(,)?) => {
        let variables = value::Variables::default();
        let doc = parser::parse_query($query_source).expect("Parse error");
        crate::validation::test_harness::expect_passes_rule_(&doc, &variables, $factory);
    };
}

pub fn expect_fails_rule_<'a, V, F>(
    doc: &'a ExecutableDocument,
    variables: &'a Variables,
    factory: F,
) where
    V: Visitor<'a> + 'a,
    F: Fn() -> V,
{
    if validate(doc, variables, factory).is_ok() {
        panic!("Expected rule to fail, but no errors were found");
    }
}

macro_rules! expect_fails_rule {
    ($factory:expr, $query_source:literal $(,)?) => {
        let variables = value::Variables::default();
        let doc = parser::parse_query($query_source).expect("Parse error");
        crate::validation::test_harness::expect_fails_rule_(&doc, &variables, $factory);
    };
}
