use std::fmt::{Debug, Display, Formatter};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use pest::iterators::Pair;
use pest::Span;

use crate::Rule;

pub mod abs;
pub mod conc;

#[derive(Debug, Copy, Clone)]
pub struct Loc {
    pub line: usize,
    pub col: usize,
    pub start: usize,
    pub end: usize,
}

impl Display for Loc {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{}:{}", self.line, self.col).as_str())
    }
}

impl<'a> From<Span<'a>> for Loc {
    fn from(span: Span) -> Self {
        let line_col = span.start_pos().line_col();
        Loc {
            line: line_col.0,
            col: line_col.1,
            start: span.start(),
            end: span.end(),
        }
    }
}

type Name = Rc<String>;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LocalVar {
    pub name: Name,
}

impl Display for LocalVar {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name.as_str())
    }
}

impl<'a> From<Pair<'a, Rule>> for LocalVar {
    fn from(p: Pair<'a, Rule>) -> Self {
        Self::new(p.as_str())
    }
}

impl LocalVar {
    fn new<S: AsRef<str>>(name: S) -> Self {
        LocalVar {
            name: Rc::new(name.as_ref().to_string()),
        }
    }

    pub fn unbound() -> Self {
        Self::new("_")
    }

    pub fn tupled() -> Self {
        Self::new("_tupled")
    }

    pub fn id(&self) -> usize {
        Rc::as_ptr(&self.name) as _
    }
}

impl Hash for LocalVar {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state)
    }
}

pub trait Syntax {}

#[derive(Debug, Clone)]
pub struct Param<T: Syntax> {
    var: LocalVar,
    typ: Box<T>,
}
