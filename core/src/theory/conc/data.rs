use std::fmt::{Display, Formatter};

use crate::theory::abs::data::Dir;
use crate::theory::{Loc, Param, Syntax, Var};

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ArgInfo {
    UnnamedExplicit,
    UnnamedImplicit,
    NamedImplicit(String),
}

#[derive(Debug, Clone)]
pub enum Expr {
    Unresolved(Loc, Var),
    Resolved(Loc, Var),

    Hole(Loc),
    InsertedHole(Loc),

    Let(Loc, Var, Option<Box<Self>>, Box<Self>, Box<Self>),

    Univ(Loc),

    Pi(Loc, Param<Self>, Box<Self>),
    TupledLam(Loc, Vec<Self>, Box<Self>),
    Lam(Loc, Var, Box<Self>),
    App(Loc, Box<Self>, ArgInfo, Box<Self>),

    Sigma(Loc, Param<Self>, Box<Self>),
    Tuple(Loc, Box<Self>, Box<Self>),
    TupleLet(Loc, Var, Var, Box<Self>, Box<Self>),

    Unit(Loc),
    TT(Loc),
    UnitLet(Loc, Box<Self>, Box<Self>),

    Boolean(Loc),
    False(Loc),
    True(Loc),
    If(Loc, Box<Self>, Box<Self>, Box<Self>),

    String(Loc),
    Str(Loc, String),

    Number(Loc),
    Num(Loc, String),

    BigInt(Loc),
    Big(Loc, String),

    Row(Loc),
    Fields(Loc, Vec<(String, Self)>),
    Combine(Loc, Box<Self>, Box<Self>),

    RowOrd(Loc, Box<Self>, Dir, Box<Self>),
    RowSat(Loc),

    RowEq(Loc, Box<Self>, Box<Self>),
    RowRefl(Loc),

    Object(Loc, Box<Self>),
    Obj(Loc, Box<Self>),
    Concat(Loc, Box<Self>, Box<Self>),
    Access(Loc, String),
    Cast(Loc, Box<Self>),

    Enum(Loc, Box<Self>),
    Variant(Loc, String, Box<Self>),
}

impl Expr {
    pub fn loc(&self) -> Loc {
        use Expr::*;

        match self {
            Unresolved(loc, _) => loc,
            Resolved(loc, _) => loc,
            Hole(loc) => loc,
            InsertedHole(loc) => loc,
            Let(loc, _, _, _, _) => loc,
            Univ(loc) => loc,
            Pi(loc, _, _) => loc,
            TupledLam(loc, _, _) => loc,
            Lam(loc, _, _) => loc,
            App(loc, _, _, _) => loc,
            Sigma(loc, _, _) => loc,
            Tuple(loc, _, _) => loc,
            TupleLet(loc, _, _, _, _) => loc,
            Unit(loc) => loc,
            TT(loc) => loc,
            UnitLet(loc, _, _) => loc,
            Boolean(loc) => loc,
            False(loc) => loc,
            True(loc) => loc,
            If(loc, _, _, _) => loc,
            String(loc) => loc,
            Str(loc, _) => loc,
            Number(loc) => loc,
            Num(loc, _) => loc,
            BigInt(loc) => loc,
            Big(loc, _) => loc,
            Row(loc) => loc,
            Fields(loc, _) => loc,
            Combine(loc, _, _) => loc,
            RowOrd(loc, _, _, _) => loc,
            RowSat(loc) => loc,
            RowEq(loc, _, _) => loc,
            RowRefl(loc) => loc,
            Object(loc, _) => loc,
            Obj(loc, _) => loc,
            Concat(loc, _, _) => loc,
            Access(loc, _) => loc,
            Cast(loc, _) => loc,
            Enum(loc, _) => loc,
            Variant(loc, _, _) => loc,
        }
        .clone()
    }

    pub fn wrap_tuple_lets(loc: Loc, x: &Var, vars: Vec<Self>, b: Box<Self>) -> Box<Self> {
        use Expr::*;

        let mut untupled_vars = Vec::default();
        for x in vars.iter().rev() {
            untupled_vars.push(match x {
                Unresolved(l, r) => Unresolved(l.clone(), r.untupled_right()),
                _ => unreachable!(),
            });
        }
        untupled_vars.push(Unresolved(loc, x.clone()));

        let mut wrapped = b;
        for (i, v) in vars.into_iter().rev().enumerate() {
            let (loc, lhs, rhs) = match (v, untupled_vars.get(i).unwrap()) {
                (Unresolved(loc, lhs), Unresolved(_, rhs)) => (loc, lhs, rhs),
                _ => unreachable!(),
            };
            let tm = untupled_vars.get(i + 1).unwrap();
            wrapped = Box::new(TupleLet(
                loc,
                lhs,
                rhs.clone(),
                Box::new(tm.clone()),
                wrapped,
            ));
        }

        wrapped
    }

    pub fn holed_app(f: Box<Self>) -> Box<Self> {
        use ArgInfo::*;
        use Expr::*;
        let loc = f.loc();
        Box::new(App(loc, f, UnnamedImplicit, Box::new(InsertedHole(loc))))
    }
}

impl Syntax for Expr {}

impl Display for Expr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use ArgInfo::*;
        use Expr::*;

        f.write_str(
            match self {
                Unresolved(_, r) => r.to_string(),
                Resolved(_, r) => r.to_string(),
                Hole(_) => "?".to_string(),
                InsertedHole(_) => "?".to_string(),
                Let(_, v, typ, a, b) => {
                    if let Some(ty) = typ {
                        format!("let {v}: {ty} = {a}; {b}")
                    } else {
                        format!("let {v} = {a}; {b}")
                    }
                }
                Univ(_) => "type".to_string(),
                Pi(_, p, b) => format!("{} -> {}", p, b),
                TupledLam(_, vs, b) => format!(
                    "({}) => {b}",
                    vs.into_iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                Lam(_, v, b) => format!("{v} => {b}"),
                App(_, f, i, x) => match i {
                    UnnamedExplicit => format!("({f} {x})"),
                    UnnamedImplicit => format!("({f} {{{x}}})"),
                    NamedImplicit(r) => format!("({f} {{{r} = {x}}}"),
                },
                Sigma(_, p, b) => format!("{p} * {b}"),
                Tuple(_, a, b) => format!("({a}, {b})"),
                TupleLet(_, x, y, a, b) => format!("let ({x}, {y}) = {a}; {b}"),
                Unit(_) => "unit".to_string(),
                TT(_) => "()".to_string(),
                UnitLet(_, a, b) => format!("let _ = {a}; {b}"),
                Boolean(_) => "boolean".to_string(),
                False(_) => "false".to_string(),
                True(_) => "true".to_string(),
                If(_, p, t, e) => format!("if {p} {{ {t} }} else {{ {e} }}"),
                String(_) => "string".to_string(),
                Str(_, v) => v.clone(),
                Number(_) => "number".to_string(),
                Num(_, v) => v.clone(),
                BigInt(_) => "bigint".to_string(),
                Big(_, v) => v.clone(),
                Row(_) => "row".to_string(),
                Fields(_, fields) => format!(
                    "({})",
                    fields
                        .into_iter()
                        .map(|(n, t)| format!("{n}: {t}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                Combine(_, a, b) => format!("{a} + {b}"),
                RowOrd(_, a, dir, b) => format!("{a} {dir} {b}"),
                RowSat(_) => "sat".to_string(),
                RowEq(_, a, b) => format!("{a} = {b}"),
                RowRefl(_) => "refl".to_string(),
                Object(_, r) => format!("{{{r}}}"),
                Obj(_, r) => format!("{{{r}}}"),
                Concat(_, a, b) => format!("{a}...{b}"),
                Access(_, n) => format!(".{n}"),
                Cast(_, a) => format!("{{{a}...}}"),
                Enum(_, r) => format!("[{r}]"),
                Variant(_, n, a) => format!("{n}({a})"),
            }
            .as_str(),
        )
    }
}
