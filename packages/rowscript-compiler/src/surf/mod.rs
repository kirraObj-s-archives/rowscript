use crate::surf::diag::Diag;
use crate::surf::SurfError::ParsingError;
use rowscript_core::basis::data::Ident;
use rowscript_core::presyntax::check::CheckError;
use rowscript_core::presyntax::data::Pred::{Comb, Cont};
use rowscript_core::presyntax::data::Term::{
    Abs, App, Array, Bool, Case, Cat, If, Inj, Let, Num, PrimRef, Rec, Sel, Str, Subs, TLet, Tuple,
    Unit, Var,
};
use rowscript_core::presyntax::data::{
    Dir, Label, Pred, QualifiedType, RowPred, RowType, Scheme, SchemeBinder, Term, Type,
};
use std::collections::HashMap;
use thiserror::Error;
use tree_sitter::{Language, Node, Parser, Tree};

mod diag;

#[cfg(test)]
mod tests;

#[derive(Debug, Error)]
pub enum SurfError {
    #[error("Tree-sitter backend language error")]
    LanguageError(#[from] tree_sitter::LanguageError),
    #[error("General parsing error")]
    ParsingError(String),
    #[error("Syntax error")]
    SyntaxError { info: Diag },
    #[error("Typecheck error")]
    TypecheckError(CheckError),
}

type SurfM<T> = Result<T, SurfError>;

extern "C" {
    fn tree_sitter_rowscript() -> Language;
}

macro_rules! row_type {
    ($self:ident,$e:expr,$node:ident) => {{
        let cnt = $node.named_child_count();
        if cnt == 0 {
            return $e(RowType::Labeled(vec![]));
        }
        let n = $node.named_child(0).unwrap();
        if cnt == 1 && n.kind() == "rowTypeExpression" {
            return $e($self.row_type_expr(n));
        }
        let mut rows = vec![];
        for i in (0..$node.named_child_count()).step_by(2) {
            let ident = $node.named_child(i).unwrap();
            let typ = $node
                .named_child(i + 1)
                .map(|n| $self.type_expr(n))
                .unwrap_or(Type::Unit);
            rows.push(($self.ident(ident), typ));
        }
        $e(RowType::Labeled(rows))
    }};
}

/// Surface syntax.
#[derive(Debug)]
pub struct Surf {
    src: String,
    tree: Tree,
}

impl Surf {
    pub fn new(src: String) -> SurfM<Surf> {
        let mut parser = Parser::new();
        let lang = unsafe { tree_sitter_rowscript() };
        parser.set_language(lang)?;
        parser
            .parse(&src, None)
            .ok_or(ParsingError("unexpected empty parsing tree".to_string()))
            .and_then(|tree| {
                let node = tree.root_node();
                if node.has_error() {
                    // FIXME
                    dbg!(node.to_sexp());
                    let info = Diag::diagnose(node, "syntax error").unwrap();
                    return Err(SurfError::SyntaxError { info });
                }
                Ok(Surf { src, tree })
            })
    }

    fn text(&self, node: &Node) -> String {
        self.src[node.start_byte()..node.end_byte()].into()
    }

    fn ident(&self, node: Node) -> Ident {
        Ident::new(self.text(&node), node.start_position())
    }

    fn prim_ref(&self, node: Node) -> Term {
        PrimRef(self.ident(node), Scheme::Meta(node.start_position()))
    }

    fn expr_from_fields<const L: usize>(&self, node: Node, fields: [&str; L]) -> [Term; L] {
        fields.map(|name| self.expr(node.child_by_field_name(name).unwrap()))
    }

    pub fn to_presyntax(&self) -> Term {
        self.prog(self.tree.root_node())
    }

    fn prog(&self, node: Node) -> Term {
        node.children(&mut node.walk())
            .map(|n| self.decl(n))
            .collect::<Vec<_>>()
            .into_iter()
            .rfold(Unit, move |acc, a| match a {
                Let(name, typ, exp, _) => Let(name, typ, exp, Box::from(acc)),
                TLet(name, typ, _) => TLet(name, typ, Box::from(acc)),
                _ => unreachable!(),
            })
    }

    fn decl(&self, node: Node) -> Term {
        let decl = node.child(0).unwrap();
        match decl.kind() {
            "functionDeclaration" => self.fn_decl(decl),
            "classDeclaration" => todo!(),
            "typeAliasDeclaration" => self.type_alias_decl(decl),
            _ => unreachable!(),
        }
    }

    fn type_alias_decl(&self, node: Node) -> Term {
        let name = self.ident(node.child_by_field_name("name").unwrap());
        let typ = self.type_scheme(node.child_by_field_name("target").unwrap());
        TLet(name, typ, Box::from(Unit))
    }

    fn fn_decl(&self, node: Node) -> Term {
        let name = node.child_by_field_name("name").unwrap();
        let (arg_type, arg_idents) = self.decl_sig(node.child_by_field_name("sig").unwrap());
        let (binders, preds) = node
            .child_by_field_name("header")
            .map_or((SchemeBinder::default(), vec![]), |n| {
                self.type_scheme_header(n)
            });

        Let(
            self.ident(name),
            Scheme::Scm {
                binders,
                qualified: QualifiedType {
                    preds,
                    typ: Type::Arrow(vec![
                        arg_type,
                        node.child_by_field_name("ret")
                            .map_or(Type::Unit, |n| self.type_expr(n)),
                    ]),
                },
            },
            Box::from(self.stmt_blk(node.child_by_field_name("body").unwrap(), arg_idents)),
            Box::from(Unit),
        )
    }

    fn decl_sig(&self, node: Node) -> (Type, Vec<Ident>) {
        match node.named_child_count() {
            0 => (Type::Unit, Default::default()),
            1 => {
                let n = node.named_child(0).unwrap();
                let arg = n.named_child(0).unwrap();
                let typ = n.named_child(1).unwrap();
                (self.type_expr(typ), vec![self.ident(arg)])
            }
            _ => {
                let mut types = vec![];
                let mut args = vec![];
                node.named_children(&mut node.walk()).for_each(|n| {
                    let arg = n.named_child(0).unwrap();
                    let typ = n.named_child(1).unwrap();
                    args.push(self.ident(arg));
                    types.push(self.type_expr(typ));
                });
                (Type::Tuple(types), args)
            }
        }
    }

    fn type_scheme(&self, node: Node) -> Scheme {
        let (binders, preds) = node
            .child_by_field_name("header")
            .map_or((SchemeBinder::default(), vec![]), |n| {
                self.type_scheme_header(n)
            });
        Scheme::Scm {
            binders,
            qualified: QualifiedType {
                preds,
                typ: self.type_expr(node.children(&mut node.walk()).last().unwrap()),
            },
        }
    }

    fn type_scheme_header(&self, node: Node) -> (SchemeBinder, Vec<Pred>) {
        (
            self.type_scheme_binders(node.child_by_field_name("binders").unwrap()),
            node.child_by_field_name("predicates")
                .map_or(vec![], |n| self.type_preds(n)),
        )
    }

    fn type_scheme_binders(&self, node: Node) -> SchemeBinder {
        let mut tvars = vec![];
        let mut rvars = vec![];
        node.named_children(&mut node.walk()).for_each(|n| {
            let ident = self.ident(n);
            match n.kind() {
                "identifier" => tvars.push(ident),
                "rowVariable" => rvars.push(ident),
                _ => unreachable!(),
            }
        });
        SchemeBinder::new(tvars, rvars)
    }

    fn type_preds(&self, node: Node) -> Vec<Pred> {
        node.named_children(&mut node.walk())
            .map(|n| self.type_pred(n.named_child(0).unwrap()))
            .collect()
    }

    fn type_pred(&self, node: Node) -> Pred {
        match node.kind() {
            "rowContainment" => {
                let lhs = self.row_pred_expr(node.named_child(0).unwrap());
                let rhs = self.row_pred_expr(node.named_child(1).unwrap());
                let d = match node.child(1).unwrap().kind() {
                    "<:" => Dir::L,
                    ":>" => Dir::R,
                    _ => unreachable!(),
                };
                Cont { d, lhs, rhs }
            }
            "rowCombination" => Comb {
                lhs: self.row_pred_expr(node.named_child(0).unwrap()),
                rhs: self.row_pred_expr(node.named_child(1).unwrap()),
                result: self.row_pred_expr(node.named_child(2).unwrap()),
            },
            "parenthesizedTypePredicate" => self.type_pred(node.named_child(0).unwrap()),
            _ => unreachable!(),
        }
    }

    fn row_pred_expr(&self, node: Node) -> RowPred {
        let n = node.child(0).unwrap();
        match n.kind() {
            "rowVariable" => RowPred::Var(self.ident(n), 0),
            "rowTerms" => RowPred::Labeled(self.row_terms(n)),
            _ => unreachable!(),
        }
    }

    fn type_expr(&self, node: Node) -> Type {
        match node.named_child_count() {
            1 => self.type_term(node.named_child(0).unwrap()),
            _ => Type::Arrow(
                node.named_children(&mut node.walk())
                    .map(|n| self.type_term(n))
                    .collect::<Vec<Type>>(),
            ),
        }
    }

    fn type_term(&self, node: Node) -> Type {
        let tm = node.child(0).unwrap();
        match tm.kind() {
            "recordType" => self.record_type(tm),
            "variantType" => self.variant_type(tm),
            "arrayType" => self.array_type(tm),
            "tupleType" => self.tuple_type(tm),
            "stringType" => Type::Str,
            "numberType" => Type::Num,
            "booleanType" => Type::Bool,
            "bigintType" => Type::BigInt,
            "identifier" => Type::Var(self.ident(tm), 0),
            _ => unreachable!(),
        }
    }

    fn record_type(&self, node: Node) -> Type {
        row_type!(self, Type::Record, node)
    }

    fn variant_type(&self, node: Node) -> Type {
        row_type!(self, Type::Variant, node)
    }

    fn row_type_expr(&self, node: Node) -> RowType {
        let n = node.child(0).unwrap();
        match n.kind() {
            "rowVariable" => RowType::Var(self.ident(n), 0),
            "rowTerms" => RowType::Labeled(self.row_terms(n)),
            "rowConcatenation" => RowType::Cat(
                Box::from(self.row_type_expr(n.named_child(0).unwrap())),
                Box::from(self.row_type_expr(n.named_child(1).unwrap())),
            ),
            "parenthesizedRowTypeExpression" => self.row_type_expr(n.named_child(0).unwrap()),
            _ => unreachable!(),
        }
    }

    fn row_terms(&self, node: Node) -> Vec<(Label, Type)> {
        let mut rows = vec![];
        for i in (0..node.named_child_count()).step_by(2) {
            let ident = self.ident(node.named_child(i).unwrap());
            let typ = self.type_expr(node.named_child(i + 1).unwrap());
            rows.push((ident, typ));
        }
        rows
    }

    fn array_type(&self, node: Node) -> Type {
        Type::Array(Box::from(self.type_expr(node.named_child(0).unwrap())))
    }

    fn tuple_type(&self, node: Node) -> Type {
        Type::Tuple(
            node.named_children(&mut node.walk())
                .map(|n| self.type_expr(n))
                .collect(),
        )
    }

    fn stmt_blk(&self, node: Node, idents: Vec<Ident>) -> Term {
        node.named_child(0)
            .map_or_else(|| Abs(idents, Box::from(Unit)), |n| self.stmt(n))
    }

    fn stmt(&self, node: Node) -> Term {
        let s = node.child(0).unwrap();
        match s.kind() {
            "lexicalDeclaration" => self.lex_decl(s),
            "ifStatement" => self.if_stmt(s),
            "switchStatement" => self.switch_stmt(s),
            "tryStatement" => self.try_stmt(s),
            "doStatement" => self.do_stmt(s),
            "returnStatement" => self.ret_stmt(s),
            "throwStatement" => self.throw_stmt(s),
            _ => unreachable!(),
        }
    }

    fn lex_decl(&self, node: Node) -> Term {
        let stmt = self.stmt(node.named_children(&mut node.walk()).last().unwrap());
        (0..node.named_child_count() - 1)
            .map(|i| node.named_child(i).unwrap())
            .map(|n| self.var_decl(n))
            .rfold(stmt, |acc, a| match a {
                Let(name, typ, exp, _) => Let(name, typ, exp, Box::from(acc)),
                _ => unreachable!(),
            })
    }

    fn var_decl(&self, node: Node) -> Term {
        Let(
            self.ident(node.child_by_field_name("name").unwrap()),
            node.child_by_field_name("type").map_or_else(
                || Scheme::Meta(node.start_position()),
                |n| Scheme::new_schemeless(self.type_expr(n)),
            ),
            Box::from(self.expr(node.child_by_field_name("value").unwrap())),
            Box::from(Unit),
        )
    }

    fn if_stmt(&self, node: Node) -> Term {
        let cond = node
            .child_by_field_name("cond")
            .unwrap()
            .named_child(0)
            .unwrap();
        let then = node.child_by_field_name("then").unwrap();
        let el = node.child_by_field_name("else").unwrap();
        If(
            Box::from(self.expr(cond)),
            Box::from(self.stmt_blk(then, vec![])),
            Box::from(self.stmt_blk(el, vec![])),
        )
    }

    fn switch_stmt(&self, node: Node) -> Term {
        let arg = self.expr(
            node.child_by_field_name("value")
                .unwrap()
                .named_child(0)
                .unwrap(),
        );
        let body = self.switch_body(node.child_by_field_name("body").unwrap());
        App(Box::from(body), Box::from(arg))
    }

    fn switch_body(&self, node: Node) -> Term {
        let mut cases = HashMap::new();
        let mut default = None;
        node.named_children(&mut node.walk())
            .for_each(|n| match n.kind() {
                "switchCase" => {
                    let (name, abs) = self.switch_case(n);
                    cases.insert(name, abs);
                }
                "switchDefault" => {
                    default = Some(self.stmt(n.named_child(0).unwrap()));
                }
                _ => unreachable!(),
            });
        Case(cases, Box::from(default))
    }

    fn switch_case(&self, node: Node) -> (Ident, Term) {
        let mut vars = vec![];
        let [lbl, stmt] = ["label", "statement"].map(|f| node.child_by_field_name(f).unwrap());
        if let Some(n) = node.child_by_field_name("variable") {
            vars.push(self.ident(n));
        }
        (self.ident(lbl), Abs(vars, Box::from(self.stmt(stmt))))
    }

    fn try_stmt(&self, _node: Node) -> Term {
        todo!()
    }

    fn do_stmt(&self, _node: Node) -> Term {
        todo!()
    }

    fn ret_stmt(&self, node: Node) -> Term {
        node.named_child(0).map_or(Unit, |n| self.expr(n))
    }

    fn throw_stmt(&self, _node: Node) -> Term {
        todo!()
    }

    fn expr(&self, node: Node) -> Term {
        let e = node.child(0).unwrap();
        match e.kind() {
            "primaryExpression" => self.primary_expr(e),
            "unaryExpression" => self.unary_expr(e),
            "binaryExpression" => self.binary_expr(e),
            "ternaryExpression" => self.ternary_expr(e),
            _ => unreachable!(),
        }
    }

    fn primary_expr(&self, node: Node) -> Term {
        let e = node.child(0).unwrap();
        match e.kind() {
            "subscriptExpression" => self.subs_expr(e),
            "memberExpression" => self.member_expr(e),
            "parenthesizedExpression" => self.expr(e.named_child(0).unwrap()),
            "identifier" => Var(self.ident(e), 0),
            "number" => Num(self.text(&e)),
            "string" | "regex" => Str(self.text(&e)),
            "false" => Bool(false),
            "true" => Bool(true),
            "object" => self.obj_expr(e),
            "variant" => self.variant_expr(e),
            "array" => self.array_expr(e),
            "arrowFunction" => self.arrow_func(e),
            "callExpression" => self.call_expr(e),
            "pipelineExpression" => self.pipeline_expr(e),
            _ => unreachable!(),
        }
    }

    fn subs_expr(&self, node: Node) -> Term {
        let [o, i] = self
            .expr_from_fields(node, ["object", "index"])
            .map(Box::from);
        Subs(o, i)
    }

    fn member_expr(&self, node: Node) -> Term {
        let n = node.child_by_field_name("object").unwrap();
        Sel(
            Box::from(match n.kind() {
                "expression" => self.expr(n),
                "primaryExpression" => self.primary_expr(n),
                _ => unreachable!(),
            }),
            self.ident(node.child_by_field_name("property").unwrap()),
        )
    }

    fn obj_expr(&self, node: Node) -> Term {
        match node.named_child_count() {
            0 => Unit,
            1 => self.pair(node.named_child(0).unwrap()),
            _ => Cat(node
                .named_children(&mut node.walk())
                .map(|n| self.pair(n))
                .collect()),
        }
    }

    fn variant_expr(&self, node: Node) -> Term {
        let ident = self.ident(node.named_child(0).unwrap());
        let expr = node.named_child(1).map_or(Unit, |n| self.expr(n));
        Inj(ident, Box::from(expr))
    }

    fn pair(&self, node: Node) -> Term {
        Rec(
            self.ident(node.child_by_field_name("key").unwrap()),
            Box::from(self.expr(node.child_by_field_name("value").unwrap())),
        )
    }

    fn array_expr(&self, node: Node) -> Term {
        Array(
            node.named_children(&mut node.walk())
                .map(|n| self.expr(n))
                .collect(),
        )
    }

    fn arrow_func(&self, node: Node) -> Term {
        let x = node.child_by_field_name("parameter").unwrap();
        let idents = x
            .named_children(&mut x.walk())
            .map(|n| self.ident(n))
            .collect();
        let body = node.child_by_field_name("body").unwrap();
        match body.kind() {
            "expression" => Abs(idents, Box::from(self.expr(body))),
            "statementBlock" => self.stmt_blk(body, idents),
            _ => unreachable!(),
        }
    }

    fn call_expr(&self, node: Node) -> Term {
        let x = node.child_by_field_name("arguments").unwrap();
        App(
            Box::from(self.expr(node.child_by_field_name("function").unwrap())),
            Box::from(match x.named_child_count() {
                0 => Unit,
                1 => self.expr(x.named_child(0).unwrap()),
                _ => Tuple(
                    x.named_children(&mut x.walk())
                        .map(|n| self.expr(n))
                        .collect(),
                ),
            }),
        )
    }

    fn pipeline_expr(&self, node: Node) -> Term {
        let expr = self.expr(node.named_child(0).unwrap());
        let calls = node.named_child(1).unwrap();
        let args_node = node.named_child(2).unwrap();
        if args_node.named_child_count() == 0 {
            return calls
                .named_children(&mut calls.walk())
                .map(|n| self.ident(n))
                .fold(expr, |acc, a| App(Box::from(Var(a, 0)), Box::from(acc)));
        }
        let mut args = vec![(0..calls.named_child_count() - 1)
            .map(|i| self.ident(calls.named_child(i).unwrap()))
            .fold(expr, |acc, a| App(Box::from(Var(a, 0)), Box::from(acc)))];
        args.append(
            &mut args_node
                .named_children(&mut args_node.walk())
                .map(|n| self.expr(n))
                .collect::<Vec<Term>>(),
        );

        App(
            Box::from(Var(
                self.ident(calls.named_children(&mut calls.walk()).last().unwrap()),
                0,
            )),
            Box::from(Tuple(args)),
        )
    }

    fn unary_expr(&self, node: Node) -> Term {
        App(
            Box::from(self.prim_ref(node.child(0).unwrap())),
            Box::from(self.expr(node.child(1).unwrap())),
        )
    }

    fn binary_expr(&self, node: Node) -> Term {
        let [l, r] = self.expr_from_fields(node, ["left", "right"]);
        App(
            Box::from(self.prim_ref(node.child(1).unwrap())),
            Box::new(Tuple(vec![l, r])),
        )
    }

    fn ternary_expr(&self, node: Node) -> Term {
        let [c, t, e] = self
            .expr_from_fields(node, ["cond", "then", "else"])
            .map(Box::from);
        If(c, t, e)
    }
}
