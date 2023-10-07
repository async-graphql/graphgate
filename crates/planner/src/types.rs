use std::fmt::{Display, Formatter, Result as FmtResult};

use graphgate_schema::{KeyFields, MetaType};
use indexmap::IndexMap;
use parser::{
    types::{Directive, Field, OperationType, VariableDefinition},
    Positioned,
};
use serde::{
    ser::{SerializeSeq, SerializeStruct},
    Serialize, Serializer,
};
use value::{ConstValue, Name, Value, Variables};

use crate::plan::ResponsePath;

#[derive(Debug)]
pub struct FieldRef<'a> {
    pub field: &'a Field,
    pub selection_set: SelectionRefSet<'a>,
}

#[derive(Debug)]
pub struct RequiredRef<'a> {
    pub prefix: usize,
    pub fields: &'a KeyFields,
    pub requires: Option<&'a KeyFields>,
}

#[derive(Debug)]
pub enum SelectionRef<'a> {
    FieldRef(FieldRef<'a>),
    IntrospectionTypename,
    RequiredRef(RequiredRef<'a>),
    InlineFragment {
        type_condition: Option<&'a str>,
        selection_set: SelectionRefSet<'a>,
    },
}

#[derive(Default, Debug)]
pub struct SelectionRefSet<'a>(pub Vec<SelectionRef<'a>>);

impl<'a> Display for SelectionRefSet<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        stringify_selection_ref_set_rec(f, self)
    }
}

#[derive(Debug)]
pub struct FetchQuery<'a> {
    pub entity_type: Option<&'a str>,
    pub operation_type: OperationType,
    pub variable_definitions: VariableDefinitionsRef<'a>,
    pub selection_set: SelectionRefSet<'a>,
}

impl<'a> Display for FetchQuery<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self.entity_type {
            Some(entity_type) => {
                write!(
                    f,
                    "query($representations:[_Any!]!{}{}) {{ _entities(representations:$representations) {{ ... on {} {} }} }}",
                    if self.variable_definitions.variables.is_empty() {
                        ""
                    } else {
                        ", "
                    },
                    self.variable_definitions,
                    entity_type,
                    self.selection_set
                )
            }
            None => {
                write!(f, "{}", self.operation_type)?;
                if !self.variable_definitions.variables.is_empty() {
                    write!(f, "({})", self.variable_definitions)?;
                }
                write!(f, "\n{}", self.selection_set)
            }
        }
    }
}

impl<'a> Serialize for FetchQuery<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

fn stringify_argument(
    f: &mut Formatter<'_>,
    arguments: &[(Positioned<Name>, Positioned<Value>)],
) -> FmtResult {
    write!(f, "(")?;
    for (idx, (name, value)) in arguments.iter().enumerate() {
        if idx > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{}: {}", name.node, value.node)?;
    }
    write!(f, ")")
}

fn stringify_directive(f: &mut Formatter<'_>, directive: &Directive) -> FmtResult {
    write!(f, "@{}", directive.name.node.as_str())?;
    if !directive.arguments.is_empty() {
        stringify_argument(f, &directive.arguments)?;
    }
    Ok(())
}

fn stringify_directives(f: &mut Formatter<'_>, directives: &[Positioned<Directive>]) -> FmtResult {
    for (idx, directive) in directives.iter().enumerate() {
        if idx > 0 {
            write!(f, " ")?;
        }
        stringify_directive(f, &directive.node)?;
    }
    Ok(())
}

fn stringify_key_fields(f: &mut Formatter<'_>, prefix: usize, fields: &KeyFields) -> FmtResult {
    fn stringify_key_fields_no_prefix(f: &mut Formatter<'_>, fields: &KeyFields) -> FmtResult {
        if fields.is_empty() {
            return Ok(());
        }
        write!(f, "{{")?;
        for (idx, (field_name, children)) in fields.iter().enumerate() {
            if idx > 0 {
                write!(f, " ")?;
                write!(f, "{}", field_name)?;
                stringify_key_fields_no_prefix(f, children)?;
            }
        }
        write!(f, "}}")
    }

    for (field_name, children) in fields.iter() {
        write!(f, " __key{}_{}:{}", prefix, field_name, field_name)?;
        stringify_key_fields_no_prefix(f, children)?;
    }
    Ok(())
}

fn stringify_selection_ref_set_rec(
    f: &mut Formatter<'_>,
    selection_set: &SelectionRefSet<'_>,
) -> FmtResult {
    write!(f, "{{ ")?;
    for (idx, selection) in selection_set.0.iter().enumerate() {
        if idx > 0 {
            write!(f, " ")?;
        }

        match selection {
            SelectionRef::FieldRef(field) => {
                if let Some(alias) = &field.field.alias {
                    write!(f, "{}:", alias.node)?;
                }
                write!(f, "{}", field.field.name.node)?;
                if !field.field.arguments.is_empty() {
                    stringify_argument(f, &field.field.arguments)?;
                }
                if !field.field.directives.is_empty() {
                    write!(f, " ")?;
                    stringify_directives(f, &field.field.directives)?;
                }
                if !field.selection_set.0.is_empty() {
                    write!(f, " ")?;
                    stringify_selection_ref_set_rec(f, &field.selection_set)?;
                }
            }
            SelectionRef::IntrospectionTypename => {
                write!(f, "__typename")?;
            }
            SelectionRef::RequiredRef(require_ref) => {
                write!(f, "__key{}___typename:__typename", require_ref.prefix,)?;
                stringify_key_fields(f, require_ref.prefix, require_ref.fields)?;
                if let Some(requires) = require_ref.requires {
                    stringify_key_fields(f, require_ref.prefix, requires)?;
                }
            }
            SelectionRef::InlineFragment {
                type_condition,
                selection_set,
            } => {
                match type_condition {
                    Some(type_condition) => write!(f, "... on {} ", type_condition)?,
                    None => write!(f, "... ")?,
                }
                stringify_selection_ref_set_rec(f, selection_set)?;
            }
        }
    }
    write!(f, " }}")
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

#[derive(Debug, Default)]
pub struct VariableDefinitionsRef<'a> {
    pub variables: Vec<&'a VariableDefinition>,
}

impl<'a> VariableDefinitionsRef<'a> {
    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
    }
}

impl<'a> Serialize for VariableDefinitionsRef<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        struct VariableDefinitionRef<'a>(&'a VariableDefinition);

        impl<'a> Serialize for VariableDefinitionRef<'a> {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                let mut s = serializer.serialize_struct("VariableDefinitions", 3)?;
                s.serialize_field("name", &self.0.name.node)?;
                s.serialize_field("type", &self.0.var_type.node.to_string())?;
                s.serialize_field(
                    "defaultValue",
                    &self.0.default_value.as_ref().map(|value| &value.node),
                )?;
                s.end()
            }
        }

        let mut s = serializer.serialize_seq(None)?;
        for item in &self.variables {
            s.serialize_element(&VariableDefinitionRef(item))?;
        }
        s.end()
    }
}

impl<'a> Display for VariableDefinitionsRef<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        for (idx, variable_definition) in self.variables.iter().enumerate() {
            if idx > 0 {
                write!(f, ", ")?;
            }
            write!(
                f,
                "${}: {}",
                variable_definition.name.node, variable_definition.var_type.node
            )?;
            if let Some(default_value) = &variable_definition.default_value {
                write!(f, " = {}", default_value.node)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default, Serialize)]
#[serde(transparent)]
pub struct VariablesRef<'a> {
    pub variables: IndexMap<&'a str, &'a ConstValue>,
}

impl<'a> VariablesRef<'a> {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
    }

    pub fn to_variables(&self) -> Variables {
        let mut variables = Variables::default();
        variables.extend(
            self.variables
                .iter()
                .map(|(name, value)| (Name::new(name), ConstValue::clone(value))),
        );
        variables
    }
}
