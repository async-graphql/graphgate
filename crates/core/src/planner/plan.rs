use std::fmt::{Debug, Display, Formatter, Result as FmtResult};
use std::ops::{Deref, DerefMut};

use indexmap::IndexMap;
use value::Name;

use crate::schema::ConstValue;

#[derive(Debug)]
pub enum PlanNode<'a> {
    Sequence(SequenceNode<'a>),
    Parallel(ParallelNode<'a>),
    Introspection(IntrospectionNode),
    Fetch(FetchNode<'a>),
    Flatten(FlattenNode<'a>),
}

impl<'a> PlanNode<'a> {
    pub(crate) fn flatten(self) -> Self {
        match self {
            PlanNode::Sequence(mut node) if node.nodes.len() == 1 => node.nodes.remove(0),
            PlanNode::Parallel(mut node) if node.nodes.len() == 1 => node.nodes.remove(0),
            _ => self,
        }
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct PathSegment<'a> {
    pub name: &'a str,
    pub is_list: bool,
    pub possible_type: Option<&'a str>,
}

#[derive(Clone, Default, Hash, Eq, PartialEq)]
pub struct ResponsePath<'a>(Vec<PathSegment<'a>>);

impl<'a> Debug for ResponsePath<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        for (idx, segment) in self.0.iter().enumerate() {
            if idx > 0 {
                write!(f, ".")?;
            }
            if segment.is_list {
                write!(f, "[{}]", segment.name)?;
            } else {
                write!(f, "{}", segment.name)?;
            }
            if let Some(possible_type) = segment.possible_type {
                write!(f, "({})", possible_type)?;
            }
        }
        Ok(())
    }
}

impl<'a> Display for ResponsePath<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        Debug::fmt(self, f)
    }
}

impl<'a> Deref for ResponsePath<'a> {
    type Target = Vec<PathSegment<'a>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> DerefMut for ResponsePath<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Default, Debug)]
pub struct SequenceNode<'a> {
    pub nodes: Vec<PlanNode<'a>>,
}

#[derive(Default, Debug)]
pub struct ParallelNode<'a> {
    pub nodes: Vec<PlanNode<'a>>,
}

#[derive(Debug)]
pub struct IntrospectionDirective {
    pub name: Name,
    pub arguments: IndexMap<Name, ConstValue>,
}

#[derive(Debug)]
pub struct IntrospectionField {
    pub name: Name,
    pub alias: Option<Name>,
    pub arguments: IndexMap<Name, ConstValue>,
    pub directives: Vec<IntrospectionDirective>,
    pub selection_set: IntrospectionSelectionSet,
}

#[derive(Debug, Default)]
pub struct IntrospectionSelectionSet(pub Vec<IntrospectionField>);

#[derive(Debug)]
pub struct IntrospectionNode {
    pub selection_set: IntrospectionSelectionSet,
}

#[derive(Debug)]
pub struct FetchNode<'a> {
    pub service: &'a str,
    pub query: String,
}

#[derive(Debug)]
pub struct FlattenNode<'a> {
    pub path: ResponsePath<'a>,
    pub prefix: usize,
    pub service: &'a str,
    pub parent_type: &'a str,
    pub query: String,
}

#[derive(Debug)]
pub enum SubscribeNode<'a> {
    Subscribe {
        fetch_nodes: Vec<FetchNode<'a>>,
        query_nodes: PlanNode<'a>,
    },
    Query(PlanNode<'a>),
}
