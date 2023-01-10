use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter};

use crate::theory::abs::data::Term;
use crate::theory::abs::def::Body::Meta;
use crate::theory::abs::rename::rename;
use crate::theory::ParamInfo::Explicit;
use crate::theory::{Loc, Param, Syntax, Tele, Var};

pub type Sigma = HashMap<Var, Def<Term>>;
pub type Gamma = HashMap<Var, Box<Term>>;
pub type Rho = HashMap<Var, Box<Term>>;

pub fn gamma_to_tele(g: &Gamma) -> Tele<Term> {
    g.into_iter()
        .map(|(v, typ)| Param {
            var: v.clone(),
            info: Explicit,
            typ: typ.clone(),
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct Def<T: Syntax> {
    pub loc: Loc,
    pub name: Var,
    pub tele: Tele<T>,
    pub ret: Box<T>,
    pub body: Body<T>,
}

impl<T: Syntax> Def<T> {
    pub fn new_constant_constraint(loc: Loc, name: Var, ret: Box<T>) -> Self {
        Self {
            loc,
            name,
            tele: Default::default(),
            ret,
            body: Meta(None),
        }
    }
}

impl Def<Term> {
    pub fn to_term(&self, v: Var) -> Box<Term> {
        use Body::*;
        match &self.body {
            Fun(f) => rename(Term::lam(&self.tele, f.clone())),
            Postulate => Box::new(Term::Ref(v)),
            Alias(t) => rename(Term::lam(&self.tele, t.clone())),
            Undefined => Box::new(Term::Undef(v)),
            _ => unreachable!(),
        }
    }
}

impl<T: Syntax> Display for Def<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use Body::*;
        f.write_str(
            match &self.body {
                Fun(f) => format!(
                    "function {} {}: {} {{\n\t{}\n}}",
                    self.name,
                    Param::tele_to_string(&self.tele),
                    self.ret,
                    f
                ),
                Postulate => format!(
                    "declare {}{}: {};",
                    self.name,
                    Param::tele_to_string(&self.tele),
                    self.ret,
                ),
                Alias(t) => format!(
                    "type {}{}: {} = {};",
                    self.name,
                    Param::tele_to_string(&self.tele),
                    self.ret,
                    t,
                ),
                Class(ms, meths) => {
                    format!(
                        "class {}{} {{\n{}\n{}\n}}",
                        self.name,
                        Param::tele_to_string(&self.tele),
                        ms.iter()
                            .map(|m| format!("\t{}: {};", m.var, m.typ))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        meths
                            .iter()
                            .map(|m| m.to_string())
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    )
                }

                Undefined => format!(
                    "undefined {} {}: {}",
                    self.name,
                    Param::tele_to_string(&self.tele),
                    self.ret,
                ),
                Meta(s) => {
                    let tele = Param::tele_to_string(&self.tele);
                    if let Some(solved) = s {
                        format!(
                            "meta {} {}: {} {{\n\t{}\n}}",
                            self.name, tele, self.ret, solved
                        )
                    } else {
                        format!("meta {} {}: {};", self.name, tele, self.ret,)
                    }
                }
            }
            .as_str(),
        )
    }
}

#[derive(Clone, Debug)]
pub enum Body<T: Syntax> {
    Fun(Box<T>),
    Postulate,
    Alias(Box<T>),
    Class(Tele<T>, Vec<Method<T>>),

    Undefined,
    Meta(Option<T>),
}

#[derive(Clone, Debug)]
pub struct Method<T: Syntax> {
    loc: Loc,
    name: Var,
    tele: Tele<T>,
    ret: Box<T>,
    body: Box<T>,
}

impl<T: Syntax> From<Def<T>> for Method<T> {
    fn from(d: Def<T>) -> Self {
        use Body::*;
        match d.body {
            Fun(f) => Self {
                loc: d.loc,
                name: d.name,
                tele: d.tele,
                ret: d.ret,
                body: f,
            },
            _ => unreachable!(),
        }
    }
}

impl<T: Syntax> Display for Method<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            format!(
                "{}{}: {} {{\n\t{}\n}}",
                self.name,
                Param::tele_to_string(&self.tele),
                self.ret,
                self.body,
            )
            .as_str(),
        )
    }
}
