#![forbid(unsafe_code)]

#[cfg(test)]
#[macro_use]
mod test_harness;

mod error;
mod rules;
mod suggestion;
mod utils;
mod visitor;

pub use error::RuleError;
use graphgate_schema::ComposedSchema;
use parser::types::ExecutableDocument;
use value::Variables;
use visitor::{visit, Visitor, VisitorContext, VisitorNil};

macro_rules! rules {
    ($($rule:ident),*) => {
        VisitorNil$(.with(rules::$rule::default()))*
    };
}

pub fn check_rules(
    composed_schema: &ComposedSchema,
    document: &ExecutableDocument,
    variables: &Variables,
) -> Vec<RuleError> {
    let mut ctx = VisitorContext::new(composed_schema, document, variables);
    let mut visitor = rules!(
        ArgumentsOfCorrectType,
        DefaultValuesOfCorrectType,
        FieldsOnCorrectType,
        FragmentsOnCompositeTypes,
        KnownArgumentNames,
        KnownDirectives,
        KnownFragmentNames,
        KnownTypeNames,
        NoFragmentCycles,
        NoUndefinedVariables,
        NoUnusedVariables,
        NoUnusedFragments,
        OverlappingFieldsCanBeMerged,
        PossibleFragmentSpreads,
        ProvidedNonNullArguments,
        ScalarLeafs,
        UniqueArgumentNames,
        UniqueVariableNames,
        VariablesAreInputTypes,
        VariableInAllowedPosition
    );
    visit(&mut visitor, &mut ctx, document);
    ctx.errors
}
