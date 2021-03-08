use std::fmt::{Result as FmtResult, Write};

use indexmap::IndexMap;
use parser::types::{Directive, Field, OperationType};
use parser::Positioned;
use value::{Name, Value, Variables};

use super::plan::ResponsePath;
use crate::schema::{KeyFields, MetaType};

pub struct FieldRef<'a> {
    pub field: &'a Field,
    pub selection_set: SelectionRefSet<'a>,
}

pub struct RequiredRef<'a> {
    pub prefix: usize,
    pub fields: &'a KeyFields,
    pub requires: Option<&'a KeyFields>,
}

pub enum SelectionRef<'a> {
    FieldRef(FieldRef<'a>),
    IntrospectionTypename,
    RequiredRef(RequiredRef<'a>),
    InlineFragment {
        type_condition: Option<&'a str>,
        selection_set: SelectionRefSet<'a>,
    },
}

#[derive(Default)]
pub struct SelectionRefSet<'a>(pub Vec<SelectionRef<'a>>);

impl<'a> SelectionRefSet<'a> {
    pub fn to_query(&self, operation_type: Option<OperationType>, variables: &Variables) -> String {
        let mut s = String::new();
        if let Some(operation_type) = operation_type {
            match operation_type {
                OperationType::Query => s.push_str("query "),
                OperationType::Mutation => s.push_str("mutation "),
                OperationType::Subscription => s.push_str("subscription "),
            }
        }
        stringify_selection_ref_set_rec(&mut s, variables, self).unwrap();
        s
    }
}

fn stringify_argument(
    w: &mut String,
    variables: &Variables,
    arguments: &[(Positioned<Name>, Positioned<Value>)],
) -> FmtResult {
    write!(w, "(")?;
    for (idx, (name, value)) in arguments.iter().enumerate() {
        if idx > 0 {
            write!(w, " ")?;
        }
        match &value.node {
            Value::Variable(var_name) => {
                if let Some(value) = variables.get(var_name.as_str()) {
                    write!(w, "{}: {}", name.node, value)?;
                } else {
                    write!(w, "{}: {}", name.node, value.node)?;
                }
            }
            _ => {
                write!(w, "{}: {}", name.node, value.node)?;
            }
        }
    }
    write!(w, ")")
}

fn stringify_directive(w: &mut String, variables: &Variables, directive: &Directive) -> FmtResult {
    write!(w, "@{}", directive.name.node.as_str())?;
    if !directive.arguments.is_empty() {
        stringify_argument(w, variables, &directive.arguments)?;
    }
    Ok(())
}

fn stringify_directives(
    w: &mut String,
    variables: &Variables,
    directives: &[Positioned<Directive>],
) -> FmtResult {
    for (idx, directive) in directives.iter().enumerate() {
        if idx > 0 {
            write!(w, " ")?;
        }
        stringify_directive(w, variables, &directive.node)?;
    }
    Ok(())
}

fn stringify_key_fields(w: &mut String, prefix: usize, fields: &KeyFields) -> FmtResult {
    fn stringify_key_fields_no_prefix(w: &mut String, fields: &KeyFields) -> FmtResult {
        if fields.is_empty() {
            return Ok(());
        }
        write!(w, "{{")?;
        for (idx, (field_name, children)) in fields.iter().enumerate() {
            if idx > 0 {
                write!(w, " ")?;
                write!(w, "{}", field_name)?;
                stringify_key_fields_no_prefix(w, children)?;
            }
        }
        write!(w, "}}")
    }

    for (idx, (field_name, children)) in fields.iter().enumerate() {
        if idx > 0 {
            write!(w, " ")?;
        }
        write!(w, "__key{}_{}:{}", prefix, field_name, field_name)?;
        stringify_key_fields_no_prefix(w, &children)?;
    }
    Ok(())
}

fn stringify_selection_ref_set_rec(
    w: &mut String,
    variables: &Variables,
    selection_set: &SelectionRefSet<'_>,
) -> FmtResult {
    write!(w, "{{")?;
    for (idx, selection) in selection_set.0.iter().enumerate() {
        if idx > 0 {
            write!(w, " ")?;
        }

        match selection {
            SelectionRef::FieldRef(field) => {
                if let Some(alias) = &field.field.alias {
                    write!(w, "{}:", alias.node)?;
                }
                write!(w, "{}", field.field.name.node)?;
                if !field.field.arguments.is_empty() {
                    write!(w, " ")?;
                    stringify_argument(w, variables, &field.field.arguments)?;
                }
                if !field.field.directives.is_empty() {
                    write!(w, " ")?;
                    stringify_directives(w, variables, &field.field.directives)?;
                }
                if !field.selection_set.0.is_empty() {
                    write!(w, " ")?;
                    stringify_selection_ref_set_rec(w, variables, &field.selection_set)?;
                }
            }
            SelectionRef::IntrospectionTypename => {
                write!(w, "__typename")?;
            }
            SelectionRef::RequiredRef(require_ref) => {
                write!(
                    w,
                    " ... {{ __key{}___typename:__typename ",
                    require_ref.prefix,
                )?;
                stringify_key_fields(w, require_ref.prefix, &require_ref.fields)?;
                if let Some(requires) = require_ref.requires {
                    stringify_key_fields(w, require_ref.prefix, &requires)?;
                }
                write!(w, " }} ")?;
            }
            SelectionRef::InlineFragment {
                type_condition,
                selection_set,
            } => {
                match type_condition {
                    Some(type_condition) => write!(w, "... on {} ", type_condition)?,
                    None => write!(w, "... ")?,
                }
                stringify_selection_ref_set_rec(w, variables, selection_set)?;
            }
        }
    }
    write!(w, "}}")
}

pub trait RootGroup<'a> {
    fn selection_set_mut(&mut self, service: &'a str) -> &mut SelectionRefSet<'a>;

    fn into_selection_set(self) -> Vec<(&'a str, SelectionRefSet<'a>)>;
}

#[derive(Default)]
pub struct QueryRootGroup<'a>(IndexMap<&'a str, SelectionRefSet<'a>>);

impl<'a> RootGroup<'a> for QueryRootGroup<'a> {
    fn selection_set_mut(&mut self, service: &'a str) -> &mut SelectionRefSet<'a> {
        self.0.entry(service).or_default()
    }

    fn into_selection_set(self) -> Vec<(&'a str, SelectionRefSet<'a>)> {
        self.0.into_iter().collect()
    }
}

#[derive(Default)]
pub struct MutationRootGroup<'a>(Vec<(&'a str, SelectionRefSet<'a>)>);

impl<'a> RootGroup<'a> for MutationRootGroup<'a> {
    fn selection_set_mut(&mut self, service: &'a str) -> &mut SelectionRefSet<'a> {
        if self
            .0
            .last()
            .filter(|(last_service, _)| *last_service == service)
            .is_some()
        {
            return &mut self.0.last_mut().unwrap().1;
        }
        self.0.push((service, Default::default()));
        let last = self.0.last_mut().unwrap();
        &mut last.1
    }

    fn into_selection_set(self) -> Vec<(&'a str, SelectionRefSet<'a>)> {
        self.0
    }
}

pub struct FetchEntity<'a> {
    pub parent_type: &'a MetaType,
    pub prefix: usize,
    pub fields: Vec<&'a Field>,
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct FetchEntityKey<'a> {
    pub service: &'a str,
    pub path: ResponsePath<'a>,
    pub ty: &'a str,
}

pub type FetchEntityGroup<'a> = IndexMap<FetchEntityKey<'a>, FetchEntity<'a>>;
