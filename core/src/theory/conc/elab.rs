use crate::maybe_grow;
use crate::theory::abs::data::Dir::Le;
use crate::theory::abs::data::{CaseMap, FieldMap, MetaKind, Term};
use crate::theory::abs::def::{gamma_to_tele, Body, ClassBody, ImplementsBody};
use crate::theory::abs::def::{Def, Gamma, Sigma};
use crate::theory::abs::normalize::Normalizer;
use crate::theory::abs::rename::rename;
use crate::theory::abs::unify::Unifier;
use crate::theory::conc::data::ArgInfo::{NamedImplicit, UnnamedExplicit};
use crate::theory::conc::data::{ArgInfo, Expr};
use crate::theory::ParamInfo::{Explicit, Implicit};
use crate::theory::{Loc, Param, Tele, Var, VarGen, VPTR};
use crate::Error;
use crate::Error::{
    ExpectedClass, ExpectedEnum, ExpectedImplementsOf, ExpectedInterface, ExpectedObject,
    ExpectedPi, ExpectedSigma, FieldsUnknown, NonExhaustive, UnresolvedField,
    UnresolvedImplicitParam,
};

#[derive(Debug, Default)]
pub struct Elaborator {
    pub sigma: Sigma,
    gamma: Gamma,
    vg: VarGen,
}

impl Elaborator {
    pub fn defs(&mut self, defs: Vec<Def<Expr>>) -> Result<Vec<Def<Term>>, Error> {
        let mut ret = Vec::default();
        for d in defs {
            ret.push(self.def(d)?);
        }
        Ok(ret)
    }

    fn def(&mut self, d: Def<Expr>) -> Result<Def<Term>, Error> {
        use Body::*;

        let mut checked = Vec::default();
        let mut tele = Tele::default();
        for p in d.tele {
            let gamma_var = p.var.clone();
            let checked_var = p.var.clone();
            let var = p.var.clone();

            let gamma_typ = self.check(*p.typ, &Term::Univ)?;
            let typ = Box::new(gamma_typ.clone());

            self.gamma.insert(gamma_var, Box::new(gamma_typ));
            checked.push(checked_var);
            tele.push(Param {
                var,
                info: p.info,
                typ,
            })
        }

        let ret = self.check(*d.ret, &Term::Univ)?;
        self.sigma.insert(
            d.name.clone(),
            Def {
                loc: d.loc,
                name: d.name.clone(),
                tele,
                ret: Box::new(ret.clone()),
                body: Undefined,
            },
        );

        let mut inferred_ret = None;
        let body = match d.body {
            Fn(f) => Fn(self.check(f, &ret)?),
            Postulate => Postulate,
            Alias(t) => Alias(self.check(t, &ret)?),
            Const(anno, f) => Const(
                anno,
                if anno {
                    self.check(f, &ret)?
                } else {
                    let (tm, ty) = self.infer(f, None)?;
                    inferred_ret = Some(Box::new(ty));
                    tm
                },
            ),

            Class(body) => Class(Box::new(ClassBody {
                object: self.check(body.object, &ret)?,
                methods: body.methods,
                ctor: body.ctor,
                vptr: body.vptr,
                vptr_ctor: body.vptr_ctor,
                vtbl: body.vtbl,
                vtbl_lookup: body.vtbl_lookup,
            })),
            Ctor(f) => Ctor(self.check(f, &ret)?),
            Method(f) => Method(self.check(f, &ret)?),
            VptrType(t) => VptrType(self.check(t, &ret)?),
            VptrCtor(t) => VptrCtor(t),
            VtblType(t) => VtblType(self.check(t, &ret)?),
            VtblLookup => VtblLookup,

            Interface { fns, ims } => Interface { fns, ims },
            Implements(body) => Implements(self.check_implements_body(&d.name, *body)?),
            ImplementsFn(f) => ImplementsFn(self.check(f, &ret)?),
            Findable(i) => Findable(i),

            Undefined => unreachable!(),
            Meta(_, _) => unreachable!(),
        };

        for n in checked {
            self.gamma.remove(&n);
        }

        let mut checked = self.sigma.get_mut(&d.name).unwrap();
        checked.body = body;
        if let Some(ret) = inferred_ret {
            checked.ret = ret;
        }

        Ok(checked.clone())
    }

    fn check_implements_body(
        &mut self,
        d: &Var,
        body: ImplementsBody<Expr>,
    ) -> Result<Box<ImplementsBody<Term>>, Error> {
        use Body::*;
        use Expr::*;

        let (i, im) = body.i;
        let ret = Box::new(ImplementsBody {
            i: (i, Box::new(self.infer(*im, None)?.0)),
            fns: body.fns,
        });
        let im_tm = ret.implementor_type(&self.sigma)?;

        let i_def = self.sigma.get_mut(&ret.i.0).unwrap();
        let i_def_loc = i_def.loc;
        match &mut i_def.body {
            Interface { fns, ims, .. } => {
                ims.push(d.clone());
                for f in fns {
                    if ret.fns.contains_key(f) {
                        continue;
                    }
                    return Err(NonExhaustive(im_tm, i_def_loc));
                }
            }
            _ => return Err(ExpectedInterface(Term::Ref(ret.i.0.clone()), i_def_loc)),
        };

        for (i_fn, im_fn) in &ret.fns {
            let i_fn_def = self.sigma.get(i_fn).unwrap();

            let i_loc = i_fn_def.loc;
            let im_loc = self.sigma.get(im_fn).unwrap().loc;

            let (i_fn_ty_p, i_fn_ty_b) = match i_fn_def.to_type() {
                Term::Pi(p, b) => (p, b),
                _ => unreachable!(),
            };
            let i_fn_ty_applied = Normalizer::new(&mut self.sigma, i_loc)
                .with(&[(&i_fn_ty_p.var, &im_tm)], *i_fn_ty_b)?;
            let (_, im_fn_ty) = self.infer(Resolved(im_loc, im_fn.clone()), None)?;

            Unifier::new(&mut self.sigma, im_loc).unify(&i_fn_ty_applied, &im_fn_ty)?;
        }

        Ok(ret)
    }

    fn check(&mut self, e: Expr, ty: &Term) -> Result<Term, Error> {
        maybe_grow(move || self.check_impl(e, ty))
    }

    fn check_impl(&mut self, e: Expr, ty: &Term) -> Result<Term, Error> {
        use Expr::*;
        Ok(match e {
            Let(_, var, maybe_typ, a, b) => {
                let (tm, typ) = if let Some(t) = maybe_typ {
                    let checked_ty = self.check(*t, &Term::Univ)?;
                    (self.check(*a, &checked_ty)?, checked_ty)
                } else {
                    self.infer(*a, Some(ty))?
                };
                let param = Param {
                    var,
                    info: Explicit,
                    typ: Box::new(typ),
                };
                let body = self.guarded_check(&[&param], *b, ty)?;
                Term::Let(param, Box::new(tm), Box::new(body))
            }
            Lam(loc, var, body) => {
                let pi = Normalizer::new(&mut self.sigma, loc).term(ty.clone())?;
                match pi {
                    Term::Pi(ty_param, ty_body) => {
                        let param = Param {
                            var: var.clone(),
                            info: Explicit,
                            typ: ty_param.typ.clone(),
                        };
                        let body_type = Normalizer::new(&mut self.sigma, loc)
                            .with(&[(&ty_param.var, &Term::Ref(var))], *ty_body)?;
                        let checked_body = self.guarded_check(&[&param], *body, &body_type)?;
                        Term::Lam(param.clone(), Box::new(checked_body))
                    }
                    ty => return Err(ExpectedPi(ty, loc)),
                }
            }
            Tuple(loc, a, b) => {
                let sig = Normalizer::new(&mut self.sigma, loc).term(ty.clone())?;
                match sig {
                    Term::Sigma(ty_param, ty_body) => {
                        let a = self.check(*a, &ty_param.typ)?;
                        let body_type = Normalizer::new(&mut self.sigma, loc)
                            .with(&[(&ty_param.var, &a)], *ty_body)?;
                        let b = self.check(*b, &body_type)?;
                        Term::Tuple(Box::new(a), Box::new(b))
                    }
                    ty => return Err(ExpectedSigma(ty, loc)),
                }
            }
            TupleLet(_, x, y, a, b) => {
                let a_loc = a.loc();
                let (a, a_ty) = self.infer(*a, Some(ty))?;
                let sig = Normalizer::new(&mut self.sigma, a_loc).term(a_ty)?;
                match sig {
                    Term::Sigma(ty_param, typ) => {
                        let x = Param {
                            var: x,
                            info: Explicit,
                            typ: ty_param.typ,
                        };
                        let y = Param {
                            var: y,
                            info: Explicit,
                            typ,
                        };
                        let b = self.guarded_check(&[&x, &y], *b, ty)?;
                        Term::TupleLet(x, y, Box::new(a), Box::new(b))
                    }
                    ty => return Err(ExpectedSigma(ty, a_loc)),
                }
            }
            UnitLet(_, a, b) => Term::UnitLet(
                Box::new(self.check(*a, &Term::Unit)?),
                Box::new(self.check(*b, ty)?),
            ),
            If(_, p, t, e) => Term::If(
                Box::new(self.check(*p, &Term::Boolean)?),
                Box::new(self.check(*t, ty)?),
                Box::new(self.check(*e, ty)?),
            ),
            _ => {
                let loc = e.loc();
                let f_e = e.clone();

                let (mut inferred_tm, inferred_ty) = self.infer(e, Some(ty))?;
                let mut inferred = Normalizer::new(&mut self.sigma, loc).term(inferred_ty)?;
                let expected = Normalizer::new(&mut self.sigma, loc).term(ty.clone())?;

                if Self::is_hole_insertable(&expected) {
                    if let Some(f_e) = Self::app_insert_holes(f_e, UnnamedExplicit, &inferred)? {
                        let (new_tm, new_ty) = self.infer(f_e, Some(ty))?;
                        inferred_tm = new_tm;
                        inferred = new_ty;
                    }
                }

                Unifier::new(&mut self.sigma, loc).unify(&expected, &inferred)?;

                inferred_tm
            }
        })
    }

    fn infer(&mut self, e: Expr, hint: Option<&Term>) -> Result<(Term, Term), Error> {
        maybe_grow(move || self.infer_impl(e, hint))
    }

    fn infer_impl(&mut self, e: Expr, hint: Option<&Term>) -> Result<(Term, Term), Error> {
        use Expr::*;
        use MetaKind::*;

        Ok(match e {
            Resolved(_, v) => match self.gamma.get(&v) {
                Some(ty) => (Term::Ref(v), *ty.clone()),
                None => {
                    let d = self.sigma.get(&v).unwrap();
                    (d.to_term(v), d.to_type())
                }
            },
            Imported(_, v) => {
                let ty = self.sigma.get(&v).unwrap().to_type();
                (Term::Ref(v), ty)
            }
            Qualified(_, m, v) => {
                let ty = self.sigma.get(&v).unwrap().to_type();
                (Term::Qualified(m, v), ty)
            }
            Hole(loc) => self.insert_meta(loc, UserMeta),
            InsertedHole(loc) => self.insert_meta(loc, InsertedMeta),
            Pi(_, p, b) => {
                let (param_ty, _) = self.infer(*p.typ, hint)?;
                let param = Param {
                    var: p.var,
                    info: p.info,
                    typ: Box::new(param_ty),
                };
                let (b, b_ty) = self.guarded_infer(&[&param], *b, hint)?;
                (Term::Pi(param, Box::new(b)), b_ty)
            }
            AnnoLam(_, p, b) => {
                let (p_ty, _) = self.infer(*p.typ, hint)?;
                let param = Param {
                    var: p.var,
                    info: p.info,
                    typ: Box::new(p_ty),
                };
                let (b, b_ty) = self.guarded_infer(&[&param], *b, hint)?;
                (
                    Term::Lam(param.clone(), Box::new(b)),
                    Term::Pi(param, Box::new(b_ty)),
                )
            }
            App(_, f, ai, x) => {
                let f_loc = f.loc();
                let f_e = f.clone();
                let (f, f_ty) = self.infer(*f, hint)?;

                if let Some(f_e) = Self::app_insert_holes(*f_e, ai.clone(), &f_ty)? {
                    return self.infer(App(f_loc, Box::new(f_e), ai, x), hint);
                }

                match f_ty {
                    Term::Pi(p, b) => {
                        let x = self.guarded_check(
                            &[&Param {
                                var: p.var.clone(),
                                info: p.info,
                                typ: p.typ.clone(),
                            }],
                            *x,
                            &p.typ,
                        )?;
                        let applied_ty =
                            Normalizer::new(&mut self.sigma, f_loc).with(&[(&p.var, &x)], *b)?;
                        let applied = Normalizer::new(&mut self.sigma, f_loc).apply(
                            f,
                            p.info.into(),
                            &[x],
                        )?;
                        (applied, applied_ty)
                    }
                    ty => return Err(ExpectedPi(ty, f_loc)),
                }
            }
            Sigma(_, p, b) => {
                let (param_ty, _) = self.infer(*p.typ, hint)?;
                let param = Param {
                    var: p.var,
                    info: p.info,
                    typ: Box::new(param_ty),
                };
                let (b, b_ty) = self.guarded_infer(&[&param], *b, hint)?;
                (Term::Sigma(param, Box::new(b)), b_ty)
            }
            Tuple(_, a, b) => {
                let (a, a_ty) = self.infer(*a, hint)?;
                let (b, b_ty) = self.infer(*b, hint)?;
                (
                    Term::Tuple(Box::new(a), Box::new(b)),
                    Term::Sigma(
                        Param {
                            var: Var::unbound(),
                            info: Explicit,
                            typ: Box::new(a_ty),
                        },
                        Box::new(b_ty),
                    ),
                )
            }
            AnnoTupleLet(_, p, q, a, b) => {
                let p_ty = self.check(*p.typ, &Term::Univ)?;
                let p = Param {
                    var: p.var,
                    info: p.info,
                    typ: Box::new(p_ty),
                };
                let q_ty = self.guarded_check(&[&p], *q.typ, &Term::Univ)?;
                let q = Param {
                    var: q.var,
                    info: q.info,
                    typ: Box::new(q_ty),
                };
                let (b, b_ty) = self.guarded_infer(&[&p, &q], *b, hint)?;
                let a = self.check(*a, &Term::Sigma(p.clone(), q.typ.clone()))?;
                (Term::TupleLet(p, q, Box::new(a), Box::new(b)), b_ty)
            }
            Fields(_, fields) => {
                let mut inferred = FieldMap::default();
                for (f, e) in fields {
                    inferred.insert(f, self.infer(e, hint)?.0);
                }
                (Term::Fields(inferred), Term::Row)
            }
            Combine(_, a, b) => {
                let a = self.check(*a, &Term::Row)?;
                let b = self.check(*b, &Term::Row)?;
                (Term::Combine(Box::new(a), Box::new(b)), Term::Row)
            }
            RowOrd(_, a, d, b) => {
                let a = self.check(*a, &Term::Row)?;
                let b = self.check(*b, &Term::Row)?;
                (Term::RowOrd(Box::new(a), d, Box::new(b)), Term::Univ)
            }
            RowEq(_, a, b) => {
                let a = self.check(*a, &Term::Row)?;
                let b = self.check(*b, &Term::Row)?;
                (Term::RowEq(Box::new(a), Box::new(b)), Term::Univ)
            }
            Object(_, r) => {
                let r = self.check(*r, &Term::Row)?;
                (Term::Object(Box::new(r)), Term::Univ)
            }
            Obj(_, r) => match *r {
                Fields(_, fields) => {
                    let mut tm_fields = FieldMap::default();
                    let mut ty_fields = FieldMap::default();
                    for (n, e) in fields {
                        let (tm, ty) = self.infer(e, hint)?;
                        tm_fields.insert(n.clone(), tm);
                        ty_fields.insert(n, ty);
                    }
                    (
                        Term::Obj(Box::new(Term::Fields(tm_fields))),
                        Term::Object(Box::new(Term::Fields(ty_fields))),
                    )
                }
                _ => unreachable!(),
            },
            Concat(_, a, b) => {
                let x_loc = a.loc();
                let y_loc = b.loc();
                let (x, x_ty) = self.infer(*a, hint)?;
                let (y, y_ty) = self.infer(*b, hint)?;
                let ty = match (x_ty, y_ty) {
                    (Term::Object(rx), Term::Object(ry)) => {
                        Box::new(Term::Object(Box::new(Term::Combine(rx, ry))))
                    }
                    (Term::Object(_), y_ty) => return Err(ExpectedObject(y_ty, y_loc)),
                    (x_ty, _) => return Err(ExpectedObject(x_ty, x_loc)),
                };
                (Term::Concat(Box::new(x), Box::new(y)), *ty)
            }
            Access(_, n) => {
                let t = Var::new("T");
                let a = Var::new("'A");
                let o = Var::new("o");
                let tele = vec![
                    Param {
                        var: t.clone(),
                        info: Implicit,
                        typ: Box::new(Term::Univ),
                    },
                    Param {
                        var: a.clone(),
                        info: Implicit,
                        typ: Box::new(Term::Row),
                    },
                    Param {
                        var: o.clone(),
                        info: Explicit,
                        typ: Box::new(Term::Object(Box::new(Term::Ref(a.clone())))),
                    },
                    Param {
                        var: Var::unbound(),
                        info: Implicit,
                        typ: Box::new(Term::RowOrd(
                            Box::new(Term::Fields(FieldMap::from([(
                                n.clone(),
                                Term::Ref(t.clone()),
                            )]))),
                            Le,
                            Box::new(Term::Ref(a)),
                        )),
                    },
                ];
                (
                    rename(Term::lam(&tele, Term::Access(Box::new(Term::Ref(o)), n))),
                    rename(Term::pi(&tele, Term::Ref(t))),
                )
            }
            Downcast(loc, a) => {
                let b_ty = Normalizer::new(&mut self.sigma, loc).term(hint.unwrap().clone())?;
                let (a, a_ty) = self.infer(*a, hint)?;
                match (a_ty, b_ty) {
                    (Term::Object(from), Term::Object(to)) => {
                        let tele = vec![Param {
                            var: Var::unbound(),
                            info: Implicit,
                            typ: Box::new(Term::RowOrd(to.clone(), Le, from)),
                        }];
                        (
                            rename(Term::lam(&tele, Term::Downcast(Box::new(a), to.clone()))),
                            rename(Term::pi(&tele, Term::Object(to))),
                        )
                    }
                    (Term::Object(_), ty) => return Err(ExpectedObject(ty, loc)),
                    (ty, _) => return Err(ExpectedObject(ty, loc)),
                }
            }
            Enum(_, r) => {
                let r = self.check(*r, &Term::Row)?;
                (Term::Enum(Box::new(r)), Term::Univ)
            }
            Variant(loc, n, a) => {
                let b_ty =
                    Box::new(Normalizer::new(&mut self.sigma, loc).term(hint.unwrap().clone())?);
                let (a, a_ty) = self.infer(*a, hint)?;
                match *b_ty {
                    Term::Enum(to) => match (a_ty, *to) {
                        (from, Term::Fields(to)) => {
                            let from = FieldMap::from([(n.clone(), from)]);
                            Unifier::new(&mut self.sigma, loc).unify_fields_ord(&from, &to)?;
                            (
                                Term::Variant(Box::new(Term::Fields(FieldMap::from([(n, a)])))),
                                Term::Enum(Box::new(Term::Fields(to))),
                            )
                        }
                        (ty, _) => (
                            Term::Variant(Box::new(Term::Fields(FieldMap::from([(n.clone(), a)])))),
                            Term::Enum(Box::new(Term::Fields(FieldMap::from([(n, ty)])))),
                        ),
                    },
                    ty => return Err(ExpectedEnum(ty, loc)),
                }
            }
            Upcast(loc, a) => {
                let b_ty = Normalizer::new(&mut self.sigma, loc).term(hint.unwrap().clone())?;
                let (a, a_ty) = self.infer(*a, hint)?;
                match (a_ty, b_ty) {
                    (Term::Enum(from), Term::Enum(to)) => {
                        let tele = vec![Param {
                            var: Var::unbound(),
                            info: Implicit,
                            typ: Box::new(Term::RowOrd(from, Le, to.clone())),
                        }];
                        (
                            rename(Term::lam(&tele, Term::Upcast(Box::new(a), to.clone()))),
                            rename(Term::pi(&tele, Term::Enum(to))),
                        )
                    }
                    (Term::Enum(_), ty) => return Err(ExpectedEnum(ty, loc)),
                    (ty, _) => return Err(ExpectedEnum(ty, loc)),
                }
            }
            Switch(loc, a, cs) => {
                let ret_ty = hint.unwrap();
                let a_loc = a.loc();
                let (a, a_ty) = self.infer(*a, hint)?;
                let en = Normalizer::new(&mut self.sigma, loc).term(a_ty)?;
                match en {
                    Term::Enum(y) => match *y {
                        Term::Fields(f) => {
                            if f.len() != cs.len() {
                                return Err(NonExhaustive(Term::Fields(f), loc));
                            }
                            let mut m = CaseMap::default();
                            for (n, v, e) in cs {
                                let ty = f.get(&n).ok_or(UnresolvedField(
                                    n.clone(),
                                    Term::Fields(f.clone()),
                                    loc,
                                ))?;
                                let p = Param {
                                    var: v.clone(),
                                    info: Explicit,
                                    typ: Box::new(ty.clone()),
                                };
                                let tm = self.guarded_check(&[&p], e, ret_ty)?;
                                m.insert(n, (v, tm));
                            }
                            (Term::Switch(Box::new(a), m), ret_ty.clone())
                        }
                        y => return Err(FieldsUnknown(y, loc)),
                    },
                    en => return Err(ExpectedEnum(en, a_loc)),
                }
            }
            Lookup(loc, o, n, arg) => {
                let o_loc = o.loc();
                let f = match self.infer(*o.clone(), hint)?.1 {
                    Term::Object(f) => f,
                    tm => return Err(ExpectedClass(tm, o_loc)),
                };
                let f = match *f {
                    Term::Fields(f) => f,
                    tm => return Err(FieldsUnknown(tm, o_loc)),
                };
                let vp = match f.get(VPTR) {
                    Some(vp) => vp,
                    None => {
                        return Err(ExpectedClass(
                            Term::Object(Box::new(Term::Fields(f))),
                            o_loc,
                        ));
                    }
                };
                let v = match vp {
                    Term::Vptr(v, _) => v,
                    _ => unreachable!(),
                };
                let desugared = App(
                    loc,
                    Box::new(App(
                        loc,
                        Box::new(Access(loc, n)),
                        UnnamedExplicit,
                        Box::new(App(
                            loc,
                            Box::new(Resolved(loc, v.clone())),
                            UnnamedExplicit,
                            Box::new(App(
                                loc,
                                Box::new(Access(loc, VPTR.to_string())),
                                UnnamedExplicit,
                                o.clone(),
                            )),
                        )),
                    )),
                    UnnamedExplicit,
                    Box::new(Tuple(arg.loc(), o, arg)),
                );
                self.infer(desugared, hint)?
            }
            Vptr(_, r, ts) => {
                let mut types = Vec::default();
                for t in ts {
                    types.push(self.infer(t, hint)?.0);
                }
                (Term::Vptr(r, types), Term::Univ)
            }
            Find(_, _, f) => {
                let ty = self.sigma.get(&f).unwrap().to_type();
                (Term::Ref(f), ty)
            }
            ImplementsOf(loc, a) => {
                let (tm, ty) = self.infer(*a, hint)?;
                match tm {
                    Term::ImplementsOf(a, i) => (Term::ImplementsOf(a, i), ty),
                    tm => return Err(ExpectedImplementsOf(tm, loc)),
                }
            }

            Univ(_) => (Term::Univ, Term::Univ),
            Unit(_) => (Term::Unit, Term::Univ),
            TT(_) => (Term::TT, Term::Unit),
            Boolean(_) => (Term::Boolean, Term::Univ),
            False(_) => (Term::False, Term::Boolean),
            True(_) => (Term::True, Term::Boolean),
            String(_) => (Term::String, Term::Univ),
            Str(_, v) => (Term::Str(v), Term::String),
            Number(_) => (Term::Number, Term::Univ),
            Num(_, r) => (Term::Num(r.parse().unwrap()), Term::Number),
            BigInt(_) => (Term::BigInt, Term::Univ),
            Big(_, v) => (Term::Big(v), Term::BigInt),
            Row(_) => (Term::Row, Term::Univ),

            _ => unreachable!(),
        })
    }

    fn guarded_check(&mut self, ps: &[&Param<Term>], e: Expr, ty: &Term) -> Result<Term, Error> {
        for &p in ps {
            self.gamma.insert(p.var.clone(), p.typ.clone());
        }
        let ret = self.check(e, &ty.clone())?;
        for p in ps {
            self.gamma.remove(&p.var);
        }
        Ok(ret)
    }

    fn guarded_infer(
        &mut self,
        ps: &[&Param<Term>],
        e: Expr,
        hint: Option<&Term>,
    ) -> Result<(Term, Term), Error> {
        for &p in ps {
            self.gamma.insert(p.var.clone(), p.typ.clone());
        }
        let ret = self.infer(e, hint)?;
        for p in ps {
            self.gamma.remove(&p.var);
        }
        Ok(ret)
    }

    fn insert_meta(&mut self, loc: Loc, k: MetaKind) -> (Term, Term) {
        use Body::*;

        let ty_meta_var = self.vg.fresh();
        self.sigma.insert(
            ty_meta_var.clone(),
            Def {
                loc,
                name: ty_meta_var.clone(),
                tele: Default::default(),
                ret: Box::new(Term::Univ),
                body: Meta(k.clone(), None),
            },
        );
        let ty = Term::MetaRef(k.clone(), ty_meta_var, Default::default());

        let tm_meta_var = self.vg.fresh();
        let tele = gamma_to_tele(&self.gamma);
        let spine = Term::tele_to_spine(&tele);
        self.sigma.insert(
            tm_meta_var.clone(),
            Def {
                loc,
                name: tm_meta_var.clone(),
                tele,
                ret: Box::new(ty.clone()),
                body: Meta(k.clone(), None),
            },
        );
        (Term::MetaRef(k, tm_meta_var, spine), ty)
    }

    fn is_hole_insertable(expected: &Term) -> bool {
        match expected {
            Term::Pi(p, _) => p.info != Implicit,
            _ => true,
        }
    }

    fn app_insert_holes(f_e: Expr, i: ArgInfo, f_ty: &Term) -> Result<Option<Expr>, Error> {
        fn holes_to_insert(loc: Loc, name: String, mut ty: &Term) -> Result<usize, Error> {
            let mut ret = Default::default();
            loop {
                match ty {
                    Term::Pi(p, b) => {
                        if p.info != Implicit {
                            return Err(UnresolvedImplicitParam(name, loc));
                        }
                        if *p.var.name == name {
                            return Ok(ret);
                        }
                        ty = b;
                        ret += 1;
                    }
                    _ => unreachable!(),
                }
            }
        }

        Ok(match f_ty {
            Term::Pi(p, _) if p.info == Implicit => match i {
                UnnamedExplicit => Some(Expr::holed_app(f_e)),
                NamedImplicit(name) => match holes_to_insert(f_e.loc(), name.to_string(), f_ty)? {
                    0 => None,
                    n => Some((0..n).fold(f_e, |e, _| Expr::holed_app(e))),
                },
                _ => None,
            },
            _ => None,
        })
    }
}
