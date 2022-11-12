use std::fmt::Debug;
use std::hash::{Hash, Hasher};

use pest::iterators::Pair;
use pest::Span;
use uuid::Uuid;

use crate::Rule;

pub mod abs;
pub mod conc;

#[derive(Debug, Copy, Clone)]
pub struct LineCol {
    line: usize,
    col: usize,
}

impl<'a> From<Span<'a>> for LineCol {
    fn from(span: Span) -> Self {
        let line_col = span.start_pos().line_col();
        LineCol {
            line: line_col.0,
            col: line_col.1,
        }
    }
}

#[derive(Debug)]
pub struct LocalVar {
    id: Uuid,
    name: String,
}

impl<'a> From<Pair<'a, Rule>> for LocalVar {
    fn from(p: Pair<'a, Rule>) -> Self {
        Self::new(p.as_str())
    }
}

impl LocalVar {
    fn new(name: &str) -> Self {
        LocalVar {
            id: Uuid::new_v4(),
            name: name.to_string(),
        }
    }

    pub fn unbound() -> Self {
        Self::new("_")
    }
}

impl Hash for LocalVar {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state)
    }
}

#[derive(Debug)]
pub struct Param<T: Syntax> {
    var: LocalVar,
    typ: Box<T>,
}

pub trait Syntax {}
