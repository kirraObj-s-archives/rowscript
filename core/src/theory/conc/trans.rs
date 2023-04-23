use std::collections::HashMap;
use std::path::PathBuf;

use pest::iterators::{Pair, Pairs};

use crate::theory::abs::data::Dir;
use crate::theory::abs::def::Def;
use crate::theory::abs::def::{Body, ClassBody, ImplementsBody};
use crate::theory::conc::data::ArgInfo::{NamedImplicit, UnnamedExplicit, UnnamedImplicit};
use crate::theory::conc::data::{ArgInfo, Expr};
use crate::theory::conc::load::{Import, ImportedDefs, ImportedPkg, ModuleID};
use crate::theory::ParamInfo::{Explicit, Implicit};
use crate::theory::{Loc, Param, Tele, Var};
use crate::Rule;

pub struct Trans<'a> {
    module: &'a ModuleID,
}

impl<'a> Trans<'a> {
    pub fn new(module: &'a ModuleID) -> Self {
        Self { module }
    }

    pub fn file(&self, mut f: Pairs<Rule>) -> (Vec<Import>, Vec<Def<Expr>>) {
        let mut imports = Vec::default();
        let mut defs = Vec::default();
        for d in f.next().unwrap().into_inner() {
            match d.as_rule() {
                Rule::import_std | Rule::import_vendor | Rule::import_local => {
                    imports.push(self.import(d))
                }
                Rule::fn_def => defs.push(self.fn_def(d, None)),
                Rule::fn_postulate => defs.push(self.fn_postulate(d)),
                Rule::type_postulate => defs.push(self.type_postulate(d)),
                Rule::type_alias => defs.push(self.type_alias(d)),
                Rule::class_def => defs.extend(self.class_def(d)),
                Rule::interface_def => defs.extend(self.interface_def(d)),
                Rule::implements_def => defs.extend(self.implements_def(d)),
                Rule::EOI => break,
                _ => unreachable!(),
            }
        }
        (imports, defs)
    }

    fn import(&self, d: Pair<Rule>) -> Import {
        use ImportedDefs::*;
        use ImportedPkg::*;

        let loc = Loc::from(d.as_span());
        let mut i = d.into_inner();

        let mut modules = PathBuf::default();
        let p = i.next().unwrap();
        let item = p.as_str().to_string();
        let pkg = match p.as_rule() {
            Rule::std_pkg_id => Std(item),
            Rule::vendor_pkg_id => {
                let mut v = p.into_inner();
                Vendor(
                    v.next().unwrap().as_str().to_string(),
                    v.next().unwrap().as_str().to_string(),
                )
            }
            Rule::module_id => {
                modules.push(item);
                Root
            }
            _ => unreachable!(),
        };

        let mut importables = Vec::default();
        for p in i {
            let loc = Loc::from(p.as_span());
            let item = p.as_str().to_string();
            match p.as_rule() {
                Rule::module_id => modules.push(item),
                Rule::importable => importables.push((loc, item)),
                Rule::importable_loaded => {
                    return Import::new(loc, ModuleID::new(pkg, modules), Loaded)
                }
                _ => unreachable!(),
            };
        }

        Import::new(
            loc,
            ModuleID::new(pkg, modules),
            if importables.is_empty() {
                Qualified
            } else {
                Unqualified(importables)
            },
        )
    }

    fn fn_def(&self, f: Pair<Rule>, this: Option<(Expr, Tele<Expr>)>) -> Def<Expr> {
        use Body::*;
        use Expr::*;

        let loc = Loc::from(f.as_span());
        let mut pairs = f.into_inner();

        let name = Var::global(self.module, pairs.next().unwrap().as_str());

        let mut tele = Tele::default();
        let mut untupled = UntupledParams::new(loc);
        let mut preds = Tele::default();
        let mut ret = Box::new(Unit(loc));
        let mut body = None;

        if let Some((ty, implicits)) = this {
            untupled.push(
                loc,
                Param {
                    var: Var::this(),
                    info: Explicit,
                    typ: Box::new(self.wrap_implicit_apps(&implicits, ty)),
                },
            );
            tele.extend(implicits);
        }

        for p in pairs {
            match p.as_rule() {
                Rule::row_id => tele.push(self.row_param(p)),
                Rule::implicit_id => tele.push(self.implicit_param(p)),
                Rule::hkt_param => tele.push(self.hkt_param(p)),
                Rule::param => untupled.push(Loc::from(p.as_span()), self.param(p)),
                Rule::type_expr => ret = Box::new(self.type_expr(p)),
                Rule::fn_body => {
                    body = Some(self.fn_body(p));
                    break;
                }
                Rule::pred => preds.push(self.pred(p)),
                _ => unreachable!(),
            }
        }
        let untupled_vars = untupled.unresolved();
        let untupled_loc = untupled.0;
        let tupled_param = Param::from(untupled);
        let body = Fn(Expr::wrap_tuple_lets(
            untupled_loc,
            &tupled_param.var,
            untupled_vars,
            body.unwrap(),
        ));
        tele.push(tupled_param);
        tele.extend(preds);

        Def {
            loc,
            name,
            tele,
            ret,
            body,
        }
    }

    fn fn_postulate(&self, f: Pair<Rule>) -> Def<Expr> {
        use Body::*;
        use Expr::*;

        let loc = Loc::from(f.as_span());
        let mut pairs = f.into_inner();

        let name = Var::global(self.module, pairs.next().unwrap().as_str());

        let mut tele = Tele::default();
        let mut untupled = UntupledParams::new(loc);
        let mut ret = Box::new(Unit(loc));

        for p in pairs {
            match p.as_rule() {
                Rule::implicit_id => tele.push(self.implicit_param(p)),
                Rule::param => untupled.push(Loc::from(p.as_span()), self.param(p)),
                Rule::type_expr => ret = Box::new(self.type_expr(p)),
                _ => unreachable!(),
            }
        }
        tele.push(Param::from(untupled));

        Def {
            loc,
            name,
            tele,
            ret,
            body: Postulate,
        }
    }

    fn type_postulate(&self, t: Pair<Rule>) -> Def<Expr> {
        use Body::*;
        use Expr::*;

        let loc = Loc::from(t.as_span());
        let name = Var::global(self.module, t.into_inner().next().unwrap().as_str());
        let ret = Box::new(Univ(loc));

        Def {
            loc,
            name,
            tele: Default::default(),
            ret,
            body: Postulate,
        }
    }

    fn type_alias(&self, t: Pair<Rule>) -> Def<Expr> {
        use Body::*;
        use Expr::*;

        let loc = Loc::from(t.as_span());
        let mut pairs = t.into_inner();

        let name = Var::global(self.module, pairs.next().unwrap().as_str());
        let mut tele = Tele::default();
        let mut target = None;
        for p in pairs {
            match p.as_rule() {
                Rule::row_id => tele.push(self.row_param(p)),
                Rule::implicit_id => tele.push(self.implicit_param(p)),
                Rule::type_expr => target = Some(self.type_expr(p)),
                _ => unreachable!(),
            }
        }

        Def {
            loc,
            name,
            tele,
            ret: Box::new(Univ(loc)),
            body: Alias(target.unwrap()),
        }
    }

    fn wrap_implicit_apps(&self, implicits: &Tele<Expr>, mut e: Expr) -> Expr {
        use Expr::*;
        for p in implicits {
            let loc = p.typ.loc();
            e = App(
                loc,
                Box::new(e),
                UnnamedImplicit,
                Box::new(Unresolved(loc, p.var.clone())),
            );
        }
        e
    }

    fn class_def(&self, c: Pair<Rule>) -> Vec<Def<Expr>> {
        use Body::*;
        use Expr::*;

        let loc = Loc::from(c.as_span());
        let mut pairs = c.into_inner();

        let name = Var::global(self.module, pairs.next().unwrap().as_str());
        let vptr_name = name.vptr_type(self.module);
        let vptr_ctor_name = name.vptr_ctor(self.module);
        let ctor_name = name.ctor(self.module);
        let vtbl_name = name.vtbl_type(self.module);
        let vtbl_lookup_name = name.vtbl_lookup(self.module);

        let mut tele = Tele::default();
        let mut members = Vec::default();
        let mut method_defs = Vec::default();
        let mut methods = Vec::default();

        let mut vtbl_fields = Vec::default();
        for p in pairs {
            match p.as_rule() {
                Rule::implicit_id => tele.push(self.implicit_param(p)),
                Rule::class_member => {
                    let loc = Loc::from(p.as_span());
                    members.push((loc, self.param(p)));
                }
                Rule::class_method => {
                    let mut m = self.fn_def(p, Some((Unresolved(loc, name.clone()), tele.clone())));
                    vtbl_fields.push((m.name.to_string(), m.to_type()));

                    let meth_name = m.name.to_string();
                    let fn_name = name.method(self.module, m.name);
                    m.name = fn_name.clone();

                    m.body = match m.body {
                        Fn(f) => Method(f),
                        _ => unreachable!(),
                    };

                    methods.push((meth_name, fn_name));
                    method_defs.push(m);
                }
                _ => unreachable!(),
            }
        }

        let vptr_def = Def {
            loc,
            name: vptr_name.clone(),
            tele: tele.clone(),
            ret: Box::new(Univ(loc)),
            body: VptrType(Vptr(
                loc,
                vtbl_lookup_name.clone(),
                tele.iter()
                    .map(|p| Unresolved(loc, p.var.clone()))
                    .collect(),
            )),
        };
        let vptr_ctor_def = Def {
            loc,
            name: vptr_ctor_name.clone(),
            tele: tele.clone(),
            ret: Box::new(self.wrap_implicit_apps(&tele, Unresolved(loc, vptr_name.clone()))),
            body: VptrCtor(name.to_string()),
        };

        let mut ctor_untupled = UntupledParams::new(loc);
        let mut ty_fields = Vec::default();
        let mut tm_fields = Vec::default();
        for (loc, m) in members {
            ty_fields.push((m.var.to_string(), *m.typ.clone()));
            tm_fields.push((m.var.to_string(), Unresolved(loc, m.var.clone())));
            ctor_untupled.push(loc, m)
        }
        ty_fields.push((
            Var::vptr(self.module).to_string(),
            self.wrap_implicit_apps(&tele, Unresolved(loc, name.vptr_type(self.module))),
        ));
        tm_fields.push((
            Var::vptr(self.module).to_string(),
            self.wrap_implicit_apps(&tele, Unresolved(loc, name.vptr_ctor(self.module))),
        ));
        let object = Object(loc, Box::new(Fields(loc, ty_fields)));

        let untupled_vars = ctor_untupled.unresolved();
        let untupled_loc = ctor_untupled.0;
        let tupled_param = Param::from(ctor_untupled);
        let ctor_body = Ctor(Expr::wrap_tuple_lets(
            untupled_loc,
            &tupled_param.var,
            untupled_vars,
            Obj(loc, Box::new(Fields(loc, tm_fields))),
        ));
        let mut ctor_tele = tele.clone();
        ctor_tele.push(tupled_param);
        let ctor_def = Def {
            loc,
            name: ctor_name.clone(),
            tele: ctor_tele,
            ret: Box::new(Unresolved(loc, name.clone())),
            body: ctor_body,
        };

        let body = Class(Box::new(ClassBody {
            object,
            methods,
            ctor: ctor_name,
            vptr: vptr_name.clone(),
            vptr_ctor: vptr_ctor_name,
            vtbl: vtbl_name.clone(),
            vtbl_lookup: vtbl_lookup_name.clone(),
        }));

        let cls_def = Def {
            loc,
            name,
            tele: tele.clone(),
            ret: Box::new(Univ(loc)),
            body,
        };

        let vtbl_def = Def {
            loc,
            name: vtbl_name.clone(),
            tele: tele.clone(),
            ret: Box::new(Univ(loc)),
            body: VtblType(Object(loc, Box::new(Fields(loc, vtbl_fields)))),
        };
        let mut lookup_tele = tele.clone();
        lookup_tele.push(Param {
            var: Var::local("vp"),
            info: Explicit,
            typ: Box::new(self.wrap_implicit_apps(&tele, Unresolved(loc, vptr_name))),
        });
        let vtbl_lookup_def = Def {
            loc,
            name: vtbl_lookup_name,
            tele: lookup_tele,
            ret: Box::new(self.wrap_implicit_apps(&tele, Unresolved(loc, vtbl_name))),
            body: VtblLookup,
        };

        let mut defs = vec![
            vptr_def,
            vptr_ctor_def,
            cls_def,
            ctor_def,
            vtbl_def,
            vtbl_lookup_def,
        ];
        defs.extend(method_defs);
        defs
    }

    fn interface_def(&self, i: Pair<Rule>) -> Vec<Def<Expr>> {
        fn alias_type(loc: Loc, tele: &Tele<Expr>) -> Expr {
            Expr::pi(tele, Univ(loc))
        }

        use Body::*;
        use Expr::*;

        let loc = Loc::from(i.as_span());
        let mut pairs = i.into_inner();

        let name_pair = pairs.next().unwrap();
        let ret = Box::new(Univ(Loc::from(name_pair.as_span())));
        let name = Var::global(self.module, name_pair.as_str());

        let alias_pair = pairs.next().unwrap();
        let alias_loc = Loc::from(alias_pair.as_span());
        let alias = Var::local(alias_pair.as_str());

        let mut im_tele = Tele::default();
        let mut fn_defs = Vec::default();
        let mut fns = Vec::default();
        for p in pairs {
            match p.as_rule() {
                Rule::row_id => im_tele.push(self.row_param(p)),
                Rule::implicit_id => im_tele.push(self.implicit_param(p)),
                Rule::interface_fn => {
                    let mut d = self.fn_postulate(p);
                    let mut tele = vec![Param {
                        var: alias.clone(),
                        info: Implicit,
                        typ: Box::new(alias_type(alias_loc, &im_tele)),
                    }];
                    tele.extend(d.tele);
                    d.tele = tele;

                    d.body = Findable(name.clone());
                    fns.push(d.name.clone());
                    fn_defs.push(d);
                }
                _ => unreachable!(),
            }
        }

        let mut defs = vec![Def {
            loc,
            name,
            tele: vec![Param {
                var: alias,
                info: Implicit,
                typ: Box::new(alias_type(alias_loc, &im_tele)),
            }],
            ret,
            body: Interface {
                fns,
                ims: Default::default(),
            },
        }];
        defs.extend(fn_defs);
        defs
    }

    fn implements_def(&self, i: Pair<Rule>) -> Vec<Def<Expr>> {
        use Body::*;
        use Expr::*;

        let loc = Loc::from(i.as_span());
        let mut pairs = i.into_inner();

        let mut defs = Vec::default();

        let i = Var::global(self.module, pairs.next().unwrap().as_str());
        let im = Var::local(pairs.next().unwrap().as_str());

        let mut fns = HashMap::default();
        for p in pairs {
            let mut def = self.fn_def(p, None);
            let fn_name = def.name.implement_func(self.module, &i, &im);
            fns.insert(def.name.clone(), fn_name.clone());
            def.name = fn_name;
            def.body = match def.body {
                Fn(f) => ImplementsFn(f),
                _ => unreachable!(),
            };
            defs.push(def);
        }

        defs.push(Def {
            loc,
            name: i.implements(self.module, &im),
            tele: Default::default(),
            ret: Box::new(Univ(loc)),
            body: Implements(Box::new(ImplementsBody { i: (i, im), fns })),
        });
        defs
    }

    fn type_expr(&self, t: Pair<Rule>) -> Expr {
        use Expr::*;

        let p = t.into_inner().next().unwrap();
        let loc = Loc::from(p.as_span());
        match p.as_rule() {
            Rule::fn_type => {
                let ps = p.into_inner();
                let mut untupled = UntupledParams::new(loc);
                for fp in ps {
                    match fp.as_rule() {
                        Rule::param => untupled.push(Loc::from(fp.as_span()), self.param(fp)),
                        Rule::type_expr => {
                            return Pi(loc, Param::from(untupled), Box::new(self.type_expr(fp)));
                        }
                        _ => unreachable!(),
                    }
                }
                unreachable!()
            }
            Rule::string_type => String(loc),
            Rule::number_type => Number(loc),
            Rule::bigint_type => BigInt(loc),
            Rule::boolean_type => Boolean(loc),
            Rule::unit_type => Unit(loc),
            Rule::object_type_ref => Object(
                loc,
                Box::new(self.unresolved(p.into_inner().next().unwrap())),
            ),
            Rule::object_type_literal => Object(loc, Box::new(self.fields(p))),
            Rule::enum_type_ref => Enum(
                loc,
                Box::new(self.unresolved(p.into_inner().next().unwrap())),
            ),
            Rule::enum_type_literal => Enum(loc, Box::new(self.fields(p))),
            Rule::type_app => self.type_app(p),
            Rule::tyref => self.unresolved(p),
            Rule::paren_type_expr => self.type_expr(p.into_inner().next().unwrap()),
            Rule::hole => Hole(loc),
            _ => unreachable!(),
        }
    }

    fn type_app(&self, a: Pair<Rule>) -> Expr {
        use Expr::*;

        let mut pairs = a.into_inner();
        let f = pairs.next().unwrap();
        let f = match f.as_rule() {
            Rule::type_expr => self.type_expr(f),
            Rule::tyref => self.unresolved(f),
            _ => unreachable!(),
        };

        pairs
            .map(|arg| {
                let loc = Loc::from(arg.as_span());
                let (i, e) = match arg.as_rule() {
                    Rule::row_arg => self.row_arg(arg),
                    Rule::type_arg => self.type_arg(arg),
                    _ => unreachable!(),
                };
                (loc, i, e)
            })
            .fold(f, |a, (loc, i, x)| App(loc, Box::new(a), i, Box::new(x)))
    }

    fn pred(&self, pred: Pair<Rule>) -> Param<Expr> {
        use Expr::*;

        let p = pred.into_inner().next().unwrap();
        let loc = Loc::from(p.as_span());
        Param {
            var: Var::unbound(),
            info: Implicit,
            typ: match p.as_rule() {
                Rule::row_ord => {
                    let mut p = p.into_inner();
                    let lhs = self.row_expr(p.next().unwrap());
                    let dir = p.next().unwrap();
                    let dir = match dir.as_rule() {
                        Rule::row_le => Dir::Le,
                        Rule::row_ge => Dir::Ge,
                        _ => unreachable!(),
                    };
                    let rhs = self.row_expr(p.next().unwrap());
                    Box::new(RowOrd(loc, Box::new(lhs), dir, Box::new(rhs)))
                }
                Rule::row_eq => {
                    let mut p = p.into_inner();
                    let lhs = self.row_expr(p.next().unwrap());
                    let rhs = self.row_expr(p.next().unwrap());
                    Box::new(RowEq(loc, Box::new(lhs), Box::new(rhs)))
                }
                Rule::constraint_expr => Box::new(ImplementsOf(loc, Box::new(self.type_app(p)))),
                _ => unreachable!(),
            },
        }
    }

    fn row_expr(&self, e: Pair<Rule>) -> Expr {
        use Expr::*;

        let p = e.into_inner().next().unwrap();
        let loc = Loc::from(p.as_span());
        match p.as_rule() {
            Rule::row_concat => {
                let mut p = p.into_inner();
                let lhs = self.row_primary_expr(p.next().unwrap());
                let rhs = self.row_expr(p.next().unwrap());
                Combine(loc, Box::new(lhs), Box::new(rhs))
            }
            Rule::row_primary_expr => self.row_primary_expr(p),
            _ => unreachable!(),
        }
    }

    fn row_primary_expr(&self, e: Pair<Rule>) -> Expr {
        let p = e.into_inner().next().unwrap();
        match p.as_rule() {
            Rule::row_id => self.unresolved(p),
            Rule::paren_fields => self.fields(p),
            Rule::paren_row_expr => self.row_expr(p.into_inner().next().unwrap()),
            _ => unreachable!(),
        }
    }

    fn type_arg(&self, a: Pair<Rule>) -> (ArgInfo, Expr) {
        let mut p = a.into_inner();
        let id_or_type = p.next().unwrap();
        match id_or_type.as_rule() {
            Rule::type_expr => (UnnamedImplicit, self.type_expr(id_or_type)),
            Rule::tyref => (
                NamedImplicit(id_or_type.as_str().to_string()),
                self.type_expr(p.next().unwrap()),
            ),
            _ => unreachable!(),
        }
    }

    fn row_arg(&self, a: Pair<Rule>) -> (ArgInfo, Expr) {
        let mut p = a.into_inner();
        let id_or_fields = p.next().unwrap();
        match id_or_fields.as_rule() {
            Rule::paren_fields => (UnnamedImplicit, self.fields(id_or_fields)),
            Rule::row_id => (
                NamedImplicit(id_or_fields.as_str().to_string()),
                self.fields(p.next().unwrap()),
            ),
            _ => unreachable!(),
        }
    }

    fn fn_body(&self, b: Pair<Rule>) -> Expr {
        use Expr::*;

        let p = b.into_inner().next().unwrap();
        let loc = Loc::from(p.as_span());
        match p.as_rule() {
            Rule::fn_body_let => {
                let mut l = p.into_inner();
                let (id, typ, tm) = self.partial_let(&mut l);
                Let(
                    loc,
                    id,
                    typ,
                    Box::new(tm),
                    Box::new(self.fn_body(l.next().unwrap())),
                )
            }
            Rule::fn_body_unit_let => {
                let mut l = p.into_inner();
                UnitLet(
                    loc,
                    Box::new(self.expr(l.next().unwrap())),
                    Box::new(self.fn_body(l.next().unwrap())),
                )
            }
            Rule::fn_body_ret => p.into_inner().next().map_or(TT(loc), |p| self.expr(p)),
            _ => unreachable!(),
        }
    }

    fn expr(&self, e: Pair<Rule>) -> Expr {
        use Expr::*;

        let p = e.into_inner().next().unwrap();
        let loc = Loc::from(p.as_span());
        match p.as_rule() {
            Rule::string => Str(loc, p.into_inner().next().unwrap().as_str().to_string()),
            Rule::number => Num(loc, p.into_inner().next().unwrap().as_str().to_string()),
            Rule::bigint => Big(loc, p.as_str().to_string()),
            Rule::boolean_false => False(loc),
            Rule::boolean_true => True(loc),
            Rule::boolean_if => {
                let mut pairs = p.into_inner();
                If(
                    loc,
                    Box::new(self.expr(pairs.next().unwrap())),
                    Box::new(self.branch(pairs.next().unwrap())),
                    Box::new(self.branch(pairs.next().unwrap())),
                )
            }
            Rule::method_app => {
                let loc = Loc::from(p.as_span());
                let mut pairs = p.into_inner();

                let o = pairs.next().unwrap();
                let o = Box::new(match o.as_rule() {
                    Rule::expr => self.expr(o),
                    Rule::idref => self.unresolved(o),
                    _ => unreachable!(),
                });
                let n = pairs.next().unwrap().as_str().to_string();
                let arg = self.tupled_args(pairs.next().unwrap());

                pairs
                    .map(|arg| (Loc::from(arg.as_span()), self.tupled_args(arg)))
                    .fold(Lookup(loc, o, n, Box::new(arg)), |a, (loc, x)| {
                        App(loc, Box::new(a), UnnamedExplicit, Box::new(x))
                    })
            }
            Rule::rev_app => {
                let mut pairs = p.into_inner();
                let arg = pairs.next().unwrap();
                pairs
                    .fold(
                        (
                            Loc::from(arg.as_span()),
                            match arg.as_rule() {
                                Rule::expr => self.expr(arg),
                                Rule::idref => self.unresolved(arg),
                                _ => unreachable!(),
                            },
                        ),
                        |(loc, a), p| (Loc::from(p.as_span()), self.app(p, Some((loc, a)))),
                    )
                    .1
            }
            Rule::new_expr => {
                let mut pairs = p.into_inner();
                let cls = match self.unresolved(pairs.next().unwrap()) {
                    Unresolved(loc, v) => Unresolved(loc, v.ctor(self.module)),
                    _ => unreachable!(),
                };
                pairs
                    .map(|arg| {
                        let loc = Loc::from(arg.as_span());
                        let (i, e) = match arg.as_rule() {
                            Rule::type_arg => self.type_arg(arg),
                            Rule::args => (UnnamedExplicit, self.tupled_args(arg)),
                            _ => unreachable!(),
                        };
                        (loc, i, e)
                    })
                    .fold(cls, |a, (loc, i, x)| App(loc, Box::new(a), i, Box::new(x)))
            }
            Rule::object_literal => self.object_literal(p),
            Rule::object_concat => {
                let mut pairs = p.into_inner();
                let a = self.object_operand(pairs.next().unwrap());
                let b = self.object_operand(pairs.next().unwrap());
                Concat(loc, Box::new(a), Box::new(b))
            }
            Rule::object_access => {
                let mut pairs = p.into_inner();
                let a = self.object_operand(pairs.next().unwrap());
                let n = pairs.next().unwrap().as_str().to_string();
                App(loc, Box::new(Access(loc, n)), UnnamedExplicit, Box::new(a))
            }
            Rule::object_cast => Downcast(
                loc,
                Box::new(self.object_operand(p.into_inner().next().unwrap())),
            ),
            Rule::enum_variant => self.enum_variant(p),
            Rule::enum_cast => Upcast(
                loc,
                Box::new(self.enum_operand(p.into_inner().next().unwrap())),
            ),
            Rule::enum_switch => {
                let mut pairs = p.into_inner();
                let e = self.expr(pairs.next().unwrap().into_inner().next().unwrap());
                let mut cases = Vec::default();
                for p in pairs {
                    let mut c = p.into_inner();
                    let n = c.next().unwrap().as_str().to_string();
                    let mut v = None;
                    let mut body = None;
                    for p in c {
                        match p.as_rule() {
                            Rule::param_id => v = Some(Var::local(p.as_str())),
                            Rule::expr => body = Some(self.expr(p)),
                            _ => unreachable!(),
                        };
                    }
                    cases.push((n, v.unwrap_or(Var::unbound()), body.unwrap()));
                }
                Switch(loc, Box::new(e), cases)
            }
            Rule::lambda_expr => {
                let pairs = p.into_inner();
                let mut vars = Vec::default();
                let mut body = None;
                for p in pairs {
                    match p.as_rule() {
                        Rule::param_id => vars.push(self.unresolved(p)),
                        Rule::lambda_body => {
                            let b = p.into_inner().next().unwrap();
                            body = Some(match b.as_rule() {
                                Rule::expr => self.expr(b),
                                Rule::fn_body => self.fn_body(b),
                                _ => unreachable!(),
                            });
                            break;
                        }
                        _ => unreachable!(),
                    }
                }
                TupledLam(loc, vars, Box::new(body.unwrap()))
            }
            Rule::app => self.app(p, None),
            Rule::tt => TT(loc),
            Rule::idref => self.unresolved(p),
            Rule::paren_expr => self.expr(p.into_inner().next().unwrap()),
            Rule::hole => Hole(loc),
            _ => unreachable!(),
        }
    }

    fn branch(&self, b: Pair<Rule>) -> Expr {
        use Expr::*;

        let p = b.into_inner().next().unwrap();
        let loc = Loc::from(p.as_span());
        match p.as_rule() {
            Rule::branch_let => {
                let mut l = p.into_inner();
                let (id, typ, tm) = self.partial_let(&mut l);
                Let(
                    loc,
                    id,
                    typ,
                    Box::new(tm),
                    Box::new(self.branch(l.next().unwrap())),
                )
            }
            Rule::branch_unit_let => {
                let mut l = p.into_inner();
                UnitLet(
                    loc,
                    Box::new(self.expr(l.next().unwrap())),
                    Box::new(self.branch(l.next().unwrap())),
                )
            }
            Rule::expr => self.expr(p),
            _ => unreachable!(),
        }
    }

    fn app(&self, a: Pair<Rule>, mut rev_operand: Option<(Loc, Expr)>) -> Expr {
        use Expr::*;

        let mut pairs = a.into_inner();
        let f = pairs.next().unwrap();
        let f = match f.as_rule() {
            Rule::expr => self.expr(f),
            Rule::idref => self.unresolved(f),
            _ => unreachable!(),
        };

        pairs
            .map(|arg| {
                let loc = Loc::from(arg.as_span());
                let (i, e) = match arg.as_rule() {
                    Rule::row_arg => self.row_arg(arg),
                    Rule::type_arg => self.type_arg(arg),
                    Rule::args => {
                        let mut args = self.tupled_args(arg);
                        if let Some((loc, a)) = rev_operand.clone() {
                            args = Tuple(loc, Box::new(a), Box::new(args));
                        }
                        rev_operand = None; // only guarantee first reverse application
                        (UnnamedExplicit, args)
                    }
                    _ => unreachable!(),
                };
                (loc, i, e)
            })
            .fold(f, |a, (loc, i, x)| App(loc, Box::new(a), i, Box::new(x)))
    }

    fn unresolved(&self, p: Pair<Rule>) -> Expr {
        use Expr::*;
        Unresolved(Loc::from(p.as_span()), Var::local(p.as_str()))
    }

    fn row_param(&self, p: Pair<Rule>) -> Param<Expr> {
        use Expr::*;
        let loc = Loc::from(p.as_span());
        Param {
            var: Var::local(p.as_str()),
            info: Implicit,
            typ: Box::new(Row(loc)),
        }
    }

    fn implicit_param(&self, p: Pair<Rule>) -> Param<Expr> {
        use Expr::*;
        let loc = Loc::from(p.as_span());
        Param {
            var: Var::local(p.as_str()),
            info: Implicit,
            typ: Box::new(Univ(loc)),
        }
    }

    fn hkt_param(&self, p: Pair<Rule>) -> Param<Expr> {
        use Expr::*;
        let mut pairs = p.into_inner();
        let var = Var::local(pairs.next().unwrap().as_str());
        let kind = Box::new(Univ(Loc::from(pairs.next().unwrap().as_span())));
        let typ = pairs.fold(kind, |a, b| {
            let loc = Loc::from(b.as_span());
            let p = Param {
                var: Var::unbound(),
                info: Implicit,
                typ: Box::new(Univ(loc)),
            };
            Box::new(Pi(loc, p, a))
        });
        Param {
            var,
            info: Implicit,
            typ,
        }
    }

    fn param(&self, p: Pair<Rule>) -> Param<Expr> {
        let mut pairs = p.into_inner();
        Param {
            var: Var::local(pairs.next().unwrap().as_str()),
            info: Explicit,
            typ: Box::new(self.type_expr(pairs.next().unwrap())),
        }
    }

    fn fields(&self, p: Pair<Rule>) -> Expr {
        use Expr::*;

        let loc = Loc::from(p.as_span());

        let mut fields = Vec::default();
        for pair in p.into_inner() {
            let mut f = pair.into_inner();
            let id = f.next().unwrap().as_str().to_string();
            let typ = f.next().map_or(Unit(loc), |p| self.type_expr(p));
            fields.push((id, typ));
        }

        Fields(loc, fields)
    }

    fn label(&self, l: Pair<Rule>) -> (String, Expr) {
        let mut p = l.into_inner();
        (
            p.next().unwrap().as_str().to_string(),
            self.expr(p.next().unwrap()),
        )
    }

    fn object_literal(&self, l: Pair<Rule>) -> Expr {
        use Expr::*;
        let loc = Loc::from(l.as_span());
        Obj(
            loc,
            Box::new(Fields(loc, l.into_inner().map(|p| self.label(p)).collect())),
        )
    }

    fn object_operand(&self, o: Pair<Rule>) -> Expr {
        let p = o.into_inner().next().unwrap();
        match p.as_rule() {
            Rule::app => self.app(p, None),
            Rule::object_literal => self.object_literal(p),
            Rule::idref => self.unresolved(p),
            Rule::paren_expr => self.expr(p.into_inner().next().unwrap()),
            _ => unreachable!(),
        }
    }

    fn enum_variant(&self, v: Pair<Rule>) -> Expr {
        use Expr::*;
        let loc = Loc::from(v.as_span());
        let mut pairs = v.into_inner();
        let n = pairs.next().unwrap().as_str().to_string();
        let a = pairs
            .next()
            .map_or(TT(loc), |p| self.expr(p.into_inner().next().unwrap()));
        Variant(loc, n, Box::new(a))
    }

    fn enum_operand(&self, o: Pair<Rule>) -> Expr {
        let p = o.into_inner().next().unwrap();
        match p.as_rule() {
            Rule::app => self.app(p, None),
            Rule::enum_variant => self.enum_variant(p),
            Rule::idref => self.unresolved(p),
            Rule::paren_expr => self.expr(p.into_inner().next().unwrap()),
            _ => unreachable!(),
        }
    }

    fn tupled_args(&self, a: Pair<Rule>) -> Expr {
        use Expr::*;
        let loc = Loc::from(a.as_span());
        a.into_inner()
            .map(|pair| match pair.as_rule() {
                Rule::expr => (Loc::from(pair.as_span()), self.expr(pair)),
                _ => unreachable!(),
            })
            .rfold(TT(loc), |a, (loc, x)| Tuple(loc, Box::new(x), Box::new(a)))
    }

    fn partial_let(&self, pairs: &mut Pairs<Rule>) -> (Var, Option<Box<Expr>>, Expr) {
        let id = Var::local(pairs.next().unwrap().as_str());
        let mut typ = None;
        let type_or_expr = pairs.next().unwrap();
        let tm = match type_or_expr.as_rule() {
            Rule::type_expr => {
                typ = Some(Box::new(self.type_expr(type_or_expr)));
                self.expr(pairs.next().unwrap())
            }
            Rule::expr => self.expr(type_or_expr),
            _ => unreachable!(),
        };
        (id, typ, tm)
    }
}

struct UntupledParams(Loc, Vec<(Loc, Param<Expr>)>);

impl UntupledParams {
    fn new(loc: Loc) -> Self {
        Self(loc, Default::default())
    }

    fn push(&mut self, loc: Loc, param: Param<Expr>) {
        self.1.push((loc, param))
    }

    fn unresolved(&self) -> Vec<Expr> {
        use Expr::*;
        self.1
            .iter()
            .map(|(loc, p)| Unresolved(*loc, p.var.clone()))
            .collect()
    }
}

impl From<UntupledParams> for Param<Expr> {
    fn from(ps: UntupledParams) -> Self {
        use Expr::*;
        let mut ret = Unit(ps.0);
        for p in ps.1.into_iter().rev() {
            ret = Sigma(p.0, p.1, Box::new(ret));
        }
        Self {
            var: Var::tupled(),
            info: Explicit,
            typ: Box::new(ret),
        }
    }
}
