//! Recursive-descent parser for the GDScript the game and its mods use.

use std::sync::Arc;

use crate::ast::*;
use crate::lexer::{lex, Tok, Token};
use crate::Error;

pub fn parse(path: &str, src: &str) -> Result<Script, Error> {
    let toks = lex(src)?;
    let mut p = Parser { toks, pos: 0 };
    let mut script = Script { path: path.to_string(), ..Default::default() };
    p.script_body(&mut script, false)?;
    Ok(script)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

type R<T> = Result<T, Error>;

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos.min(self.toks.len() - 1)].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }

    fn line(&self) -> usize {
        self.toks[self.pos.min(self.toks.len() - 1)].line
    }

    fn next(&mut self) -> Tok {
        let t = self.peek().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn err<T>(&self, msg: impl Into<String>) -> R<T> {
        Err(Error::Parse { line: self.line(), msg: msg.into() })
    }

    fn is_op(&self, op: &str) -> bool {
        matches!(self.peek(), Tok::Op(o) if *o == op)
    }

    fn eat_op(&mut self, op: &str) -> bool {
        if self.is_op(op) {
            self.next();
            true
        } else {
            false
        }
    }

    fn expect_op(&mut self, op: &str) -> R<()> {
        if self.eat_op(op) {
            Ok(())
        } else {
            self.err(format!("expected `{op}`, found {:?}", self.peek()))
        }
    }

    fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Ident(i) if i == kw)
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.is_kw(kw) {
            self.next();
            true
        } else {
            false
        }
    }

    fn ident(&mut self) -> R<String> {
        match self.next() {
            Tok::Ident(s) => Ok(s),
            t => self.err(format!("expected a name, found {t:?}")),
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Tok::Newline) {
            self.next();
        }
    }

    // ---- declarations ---------------------------------------------------------------------

    /// The declarations of a script (or of an inner class when `nested`, until its dedent).
    fn script_body(&mut self, s: &mut Script, nested: bool) -> R<()> {
        loop {
            self.skip_newlines();
            match self.peek() {
                Tok::Eof => return Ok(()),
                Tok::Dedent if nested => {
                    self.next();
                    return Ok(());
                }
                _ => {}
            }
            self.declaration(s)?;
        }
    }

    fn declaration(&mut self, s: &mut Script) -> R<()> {
        // a docstring or stray literal between declarations
        if matches!(self.peek(), Tok::Str(_)) {
            self.next();
            return self.end_line();
        }
        // `tool` stands alone on its line
        if self.is_kw("tool") && matches!(self.peek_at(1), Tok::Newline | Tok::Eof) {
            self.next();
            return self.end_line();
        }
        // prefixes that change nothing here
        while matches!(self.peek(), Tok::Ident(i) if matches!(i.as_str(), "tool" | "remote" | "master" | "puppet" | "sync" | "remotesync" | "mastersync" | "puppetsync" | "slave"))
            && !matches!(self.peek_at(1), Tok::Op("=") | Tok::Op("."))
        {
            self.next();
        }
        let line = self.line();
        let Tok::Ident(kw) = self.peek().clone() else { return self.err(format!("unexpected {:?}", self.peek())) };
        match kw.as_str() {
            "extends" => {
                self.next();
                s.extends = Some(match self.next() {
                    Tok::Str(p) => Extends::Path(p),
                    Tok::Ident(c) => {
                        let mut name = c;
                        while self.eat_op(".") {
                            name = self.ident()?; // `extends A.B`: the innermost name
                        }
                        Extends::Class(name)
                    }
                    t => return self.err(format!("bad extends {t:?}")),
                });
                self.end_line()
            }
            "class_name" => {
                self.next();
                s.class_name = Some(self.ident()?);
                while !matches!(self.peek(), Tok::Newline | Tok::Eof) {
                    self.next(); // `, "icon.png"`
                }
                self.end_line()
            }
            "signal" => {
                self.next();
                s.signals.push(self.ident()?);
                if self.is_op("(") {
                    self.skip_parens()?;
                }
                self.end_line()
            }
            "enum" => {
                self.next();
                let name = if let Tok::Ident(n) = self.peek().clone() {
                    self.next();
                    Some(n)
                } else {
                    None
                };
                self.expect_op("{")?;
                let mut vals = Vec::new();
                while !self.is_op("}") {
                    let n = self.ident()?;
                    let v = if self.eat_op("=") { Some(self.expr()?) } else { None };
                    vals.push((n, v));
                    // values are separated by commas or just by line breaks
                    self.eat_op(",");
                }
                self.expect_op("}")?;
                s.enums.push((name, vals));
                self.end_line()
            }
            "const" => {
                self.next();
                let name = self.ident()?;
                if self.eat_op(":") {
                    self.type_hint();
                }
                let init = if self.eat_op(":=") || self.eat_op("=") { self.expr()? } else { Expr::Nil };
                s.consts.push((name, init));
                self.end_line()
            }
            "export" | "onready" | "var" => {
                let (mut exported, mut onready) = (false, false);
                loop {
                    if self.eat_kw("export") {
                        exported = true;
                        if self.is_op("(") {
                            self.skip_parens()?;
                        }
                    } else if self.eat_kw("onready") {
                        onready = true;
                    } else {
                        break;
                    }
                }
                if !self.eat_kw("var") {
                    return self.err("expected `var`");
                }
                let name = self.ident()?;
                let mut inferred = false;
                if self.eat_op(":") {
                    self.type_hint();
                } else if self.is_op(":=") {
                    inferred = true;
                }
                let init = if inferred {
                    self.next();
                    Some(self.expr()?)
                } else if self.eat_op("=") {
                    Some(self.expr()?)
                } else {
                    None
                };
                if self.eat_kw("setget") {
                    while !matches!(self.peek(), Tok::Newline | Tok::Eof) {
                        self.next();
                    }
                }
                s.members.push(Member { name, init, exported, onready });
                self.end_line()
            }
            "static" | "func" => {
                let is_static = self.eat_kw("static");
                if !self.eat_kw("func") {
                    return self.err("expected `func`");
                }
                let name = self.ident()?;
                let params = self.params()?;
                if self.eat_op("->") {
                    self.type_hint();
                }
                self.expect_op(":")?;
                let body = self.block()?;
                s.funcs.insert(name.clone(), Arc::new(Func { name, params, body, is_static }));
                Ok(())
            }
            "class" => {
                self.next();
                let name = self.ident()?;
                let mut inner = Script { path: format!("{}::{name}", s.path), ..Default::default() };
                if self.eat_kw("extends") {
                    inner.extends = Some(match self.next() {
                        Tok::Str(p) => Extends::Path(p),
                        Tok::Ident(c) => Extends::Class(c),
                        _ => return self.err("bad extends"),
                    });
                }
                self.expect_op(":")?;
                self.skip_newlines();
                if matches!(self.peek(), Tok::Indent) {
                    self.next();
                    self.script_body(&mut inner, true)?;
                }
                s.inner.push((name, Arc::new(inner)));
                Ok(())
            }
            _ => Err(Error::Parse { line, msg: format!("unexpected `{kw}` at the top level") }),
        }
    }

    fn end_line(&mut self) -> R<()> {
        match self.peek() {
            Tok::Newline => {
                self.next();
                Ok(())
            }
            Tok::Op(";") => {
                self.next();
                Ok(())
            }
            Tok::Eof | Tok::Dedent => Ok(()),
            t => self.err(format!("expected the end of the line, found {t:?}")),
        }
    }

    /// Skips a balanced `( ... )`.
    fn skip_parens(&mut self) -> R<()> {
        self.expect_op("(")?;
        let mut depth = 1;
        while depth > 0 {
            match self.next() {
                Tok::Op("(") => depth += 1,
                Tok::Op(")") => depth -= 1,
                Tok::Eof => return self.err("unclosed parenthesis"),
                _ => {}
            }
        }
        Ok(())
    }

    /// `int`, `Array`, `Vector2`, `Dictionary`... (ignored).
    fn type_hint(&mut self) {
        if matches!(self.peek(), Tok::Ident(_)) {
            self.next();
            while self.is_op(".") {
                self.next();
                if matches!(self.peek(), Tok::Ident(_)) {
                    self.next();
                }
            }
        }
    }

    fn params(&mut self) -> R<Vec<(String, Option<Expr>)>> {
        self.expect_op("(")?;
        let mut out = Vec::new();
        while !self.is_op(")") {
            let name = self.ident()?;
            if self.eat_op(":") {
                self.type_hint();
            }
            let default = if self.eat_op("=") || self.eat_op(":=") { Some(self.expr()?) } else { None };
            out.push((name, default));
            if !self.eat_op(",") {
                break;
            }
        }
        self.expect_op(")")?;
        Ok(out)
    }

    // ---- statements -----------------------------------------------------------------------

    /// After a `:`: an indented block, or the rest of the line.
    fn block(&mut self) -> R<Vec<Stmt>> {
        if matches!(self.peek(), Tok::Newline) {
            self.skip_newlines();
            if !matches!(self.peek(), Tok::Indent) {
                return Ok(vec![]);
            }
            self.next();
            let mut out = Vec::new();
            loop {
                self.skip_newlines();
                match self.peek() {
                    Tok::Dedent => {
                        self.next();
                        break;
                    }
                    Tok::Eof => break,
                    _ => out.push(self.statement()?),
                }
            }
            Ok(out)
        } else {
            let mut out = vec![self.statement()?];
            while self.eat_op(";") {
                if matches!(self.peek(), Tok::Newline | Tok::Eof) {
                    break;
                }
                out.push(self.statement()?);
            }
            Ok(out)
        }
    }

    fn statement(&mut self) -> R<Stmt> {
        let line = self.line();
        if let Tok::Ident(kw) = self.peek().clone() {
            match kw.as_str() {
                "if" => {
                    self.next();
                    let mut arms = Vec::new();
                    let cond = self.expr()?;
                    self.expect_op(":")?;
                    arms.push((cond, self.block()?));
                    let mut other = None;
                    loop {
                        self.skip_newlines();
                        if self.is_kw("elif") {
                            self.next();
                            let c = self.expr()?;
                            self.expect_op(":")?;
                            arms.push((c, self.block()?));
                        } else if self.is_kw("else") {
                            self.next();
                            self.expect_op(":")?;
                            other = Some(self.block()?);
                            break;
                        } else {
                            break;
                        }
                    }
                    return Ok(Stmt::If(arms, other));
                }
                "while" => {
                    self.next();
                    let c = self.expr()?;
                    self.expect_op(":")?;
                    return Ok(Stmt::While(c, self.block()?));
                }
                "for" => {
                    self.next();
                    let v = self.ident()?;
                    if !self.eat_kw("in") {
                        return self.err("expected `in`");
                    }
                    let it = self.expr()?;
                    self.expect_op(":")?;
                    return Ok(Stmt::For(v, it, self.block()?));
                }
                "match" => {
                    self.next();
                    let subject = self.expr()?;
                    self.expect_op(":")?;
                    self.skip_newlines();
                    let mut arms = Vec::new();
                    if matches!(self.peek(), Tok::Indent) {
                        self.next();
                        loop {
                            self.skip_newlines();
                            if matches!(self.peek(), Tok::Dedent) {
                                self.next();
                                break;
                            }
                            if matches!(self.peek(), Tok::Eof) {
                                break;
                            }
                            let mut pats = Vec::new();
                            loop {
                                pats.push(self.pattern()?);
                                if !self.eat_op(",") {
                                    break;
                                }
                            }
                            self.expect_op(":")?;
                            arms.push((pats, self.block()?));
                        }
                    }
                    return Ok(Stmt::Match(subject, arms));
                }
                "return" => {
                    self.next();
                    let v = if matches!(self.peek(), Tok::Newline | Tok::Eof | Tok::Dedent) || self.is_op(";") { None } else { Some(self.expr()?) };
                    self.end_stmt()?;
                    return Ok(Stmt::Return(v));
                }
                "break" => {
                    self.next();
                    self.end_stmt()?;
                    return Ok(Stmt::Break);
                }
                "continue" => {
                    self.next();
                    self.end_stmt()?;
                    return Ok(Stmt::Continue);
                }
                "pass" => {
                    self.next();
                    self.end_stmt()?;
                    return Ok(Stmt::Pass);
                }
                "breakpoint" => {
                    self.next();
                    self.end_stmt()?;
                    return Ok(Stmt::Pass);
                }
                "assert" => {
                    self.next();
                    self.expect_op("(")?;
                    let c = self.expr()?;
                    if self.eat_op(",") {
                        self.expr()?; // the message
                    }
                    self.expect_op(")")?;
                    self.end_stmt()?;
                    return Ok(Stmt::Assert(c));
                }
                "var" => {
                    self.next();
                    let name = self.ident()?;
                    let mut inferred = false;
                    if self.eat_op(":") {
                        self.type_hint();
                    } else if self.is_op(":=") {
                        inferred = true;
                    }
                    let init = if inferred {
                        self.next();
                        Some(self.expr()?)
                    } else if self.eat_op("=") {
                        Some(self.expr()?)
                    } else {
                        None
                    };
                    self.end_stmt()?;
                    return Ok(Stmt::Var(name, init, line));
                }
                "const" => {
                    self.next();
                    let name = self.ident()?;
                    if self.eat_op(":") {
                        self.type_hint();
                    }
                    if !self.eat_op("=") && !self.eat_op(":=") {
                        return self.err("expected `=`");
                    }
                    let v = self.expr()?;
                    self.end_stmt()?;
                    return Ok(Stmt::Const(name, v, line));
                }
                _ => {}
            }
        }
        let e = self.expr()?;
        let op = match self.peek() {
            Tok::Op("=") => Some(None),
            Tok::Op("+=") => Some(Some(BOp::Add)),
            Tok::Op("-=") => Some(Some(BOp::Sub)),
            Tok::Op("*=") => Some(Some(BOp::Mul)),
            Tok::Op("/=") => Some(Some(BOp::Div)),
            Tok::Op("%=") => Some(Some(BOp::Mod)),
            Tok::Op("**=") => Some(Some(BOp::Pow)),
            Tok::Op("&=") => Some(Some(BOp::BitAnd)),
            Tok::Op("|=") => Some(Some(BOp::BitOr)),
            Tok::Op("^=") => Some(Some(BOp::BitXor)),
            Tok::Op("<<=") => Some(Some(BOp::Shl)),
            Tok::Op(">>=") => Some(Some(BOp::Shr)),
            _ => None,
        };
        if let Some(op) = op {
            self.next();
            let value = self.expr()?;
            self.end_stmt()?;
            return Ok(Stmt::Assign { target: e, op, value, line });
        }
        self.end_stmt()?;
        Ok(Stmt::Expr(e, line))
    }

    fn end_stmt(&mut self) -> R<()> {
        if self.eat_op(";") {
            return Ok(());
        }
        match self.peek() {
            Tok::Newline => {
                self.next();
                Ok(())
            }
            Tok::Eof | Tok::Dedent => Ok(()),
            t => self.err(format!("expected the end of the statement, found {t:?}")),
        }
    }

    fn pattern(&mut self) -> R<Pattern> {
        if self.is_kw("var") {
            self.next();
            return Ok(Pattern::Bind(self.ident()?));
        }
        if matches!(self.peek(), Tok::Ident(i) if i == "_") {
            self.next();
            return Ok(Pattern::Wildcard);
        }
        Ok(Pattern::Value(self.expr()?))
    }

    // ---- expressions ----------------------------------------------------------------------

    pub fn expr(&mut self) -> R<Expr> {
        self.ternary()
    }

    fn ternary(&mut self) -> R<Expr> {
        let then = self.or()?;
        if self.is_kw("if") {
            self.next();
            let cond = self.or()?;
            if !self.eat_kw("else") {
                return self.err("expected `else`");
            }
            let other = self.ternary()?;
            return Ok(Expr::Ternary(Box::new(then), Box::new(cond), Box::new(other)));
        }
        Ok(then)
    }

    fn or(&mut self) -> R<Expr> {
        let mut l = self.and()?;
        while self.is_op("||") || self.is_kw("or") {
            self.next();
            let r = self.and()?;
            l = Expr::Binary(BOp::Or, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn and(&mut self) -> R<Expr> {
        let mut l = self.not()?;
        while self.is_op("&&") || self.is_kw("and") {
            self.next();
            let r = self.not()?;
            l = Expr::Binary(BOp::And, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn not(&mut self) -> R<Expr> {
        if self.is_op("!") || self.is_kw("not") {
            self.next();
            let e = self.not()?;
            return Ok(Expr::Unary(UOp::Not, Box::new(e)));
        }
        self.comparison()
    }

    fn comparison(&mut self) -> R<Expr> {
        let mut l = self.bit_or()?;
        loop {
            let op = match self.peek() {
                Tok::Op("==") => BOp::Eq,
                Tok::Op("!=") => BOp::Ne,
                Tok::Op("<") => BOp::Lt,
                Tok::Op(">") => BOp::Gt,
                Tok::Op("<=") => BOp::Le,
                Tok::Op(">=") => BOp::Ge,
                Tok::Ident(i) if i == "in" => BOp::In,
                Tok::Ident(i) if i == "is" => {
                    self.next();
                    let r = self.bit_or()?;
                    l = Expr::Is(Box::new(l), Box::new(r));
                    continue;
                }
                _ => break,
            };
            self.next();
            let r = self.bit_or()?;
            l = Expr::Binary(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn bit_or(&mut self) -> R<Expr> {
        let mut l = self.bit_xor()?;
        while self.is_op("|") {
            self.next();
            let r = self.bit_xor()?;
            l = Expr::Binary(BOp::BitOr, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn bit_xor(&mut self) -> R<Expr> {
        let mut l = self.bit_and()?;
        while self.is_op("^") {
            self.next();
            let r = self.bit_and()?;
            l = Expr::Binary(BOp::BitXor, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn bit_and(&mut self) -> R<Expr> {
        let mut l = self.shift()?;
        while self.is_op("&") {
            self.next();
            let r = self.shift()?;
            l = Expr::Binary(BOp::BitAnd, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn shift(&mut self) -> R<Expr> {
        let mut l = self.additive()?;
        loop {
            let op = match self.peek() {
                Tok::Op("<<") => BOp::Shl,
                Tok::Op(">>") => BOp::Shr,
                _ => break,
            };
            self.next();
            let r = self.additive()?;
            l = Expr::Binary(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn additive(&mut self) -> R<Expr> {
        let mut l = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Tok::Op("+") => BOp::Add,
                Tok::Op("-") => BOp::Sub,
                _ => break,
            };
            self.next();
            let r = self.multiplicative()?;
            l = Expr::Binary(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn multiplicative(&mut self) -> R<Expr> {
        let mut l = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Op("*") => BOp::Mul,
                Tok::Op("/") => BOp::Div,
                Tok::Op("%") => BOp::Mod,
                _ => break,
            };
            self.next();
            let r = self.unary()?;
            l = Expr::Binary(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn unary(&mut self) -> R<Expr> {
        if self.eat_op("-") {
            let e = self.unary()?;
            return Ok(match e {
                Expr::Int(i) => Expr::Int(-i),
                Expr::Float(f) => Expr::Float(-f),
                e => Expr::Unary(UOp::Neg, Box::new(e)),
            });
        }
        if self.eat_op("+") {
            return self.unary();
        }
        if self.eat_op("~") {
            return Ok(Expr::Unary(UOp::BitNot, Box::new(self.unary()?)));
        }
        self.power()
    }

    fn power(&mut self) -> R<Expr> {
        let l = self.postfix()?;
        if self.eat_op("**") {
            let r = self.unary()?;
            return Ok(Expr::Binary(BOp::Pow, Box::new(l), Box::new(r)));
        }
        // `x as Type`
        while self.is_kw("as") {
            self.next();
            self.type_hint();
        }
        Ok(l)
    }

    fn postfix(&mut self) -> R<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.eat_op(".") {
                let name = self.ident()?;
                e = Expr::Member(Box::new(e), name);
            } else if self.is_op("(") {
                let args = self.args()?;
                e = Expr::Call(Box::new(e), args);
            } else if self.eat_op("[") {
                let i = self.expr()?;
                self.expect_op("]")?;
                e = Expr::Index(Box::new(e), Box::new(i));
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn args(&mut self) -> R<Vec<Expr>> {
        self.expect_op("(")?;
        let mut out = Vec::new();
        while !self.is_op(")") {
            out.push(self.expr()?);
            if !self.eat_op(",") {
                break;
            }
        }
        self.expect_op(")")?;
        Ok(out)
    }

    fn primary(&mut self) -> R<Expr> {
        let t = self.next();
        Ok(match t {
            Tok::Int(i) => Expr::Int(i),
            Tok::Float(f) => Expr::Float(f),
            Tok::Str(s) => {
                // `"%d" % x` is handled as the binary `%`; adjacent literals do not concatenate
                Expr::Str(s)
            }
            Tok::NodePath(p) => Expr::NodePath(p),
            Tok::Op("(") => {
                let e = self.expr()?;
                self.expect_op(")")?;
                e
            }
            Tok::Op("[") => {
                let mut items = Vec::new();
                while !self.is_op("]") {
                    items.push(self.expr()?);
                    if !self.eat_op(",") {
                        break;
                    }
                }
                self.expect_op("]")?;
                Expr::Array(items)
            }
            Tok::Op("{") => {
                let mut items = Vec::new();
                while !self.is_op("}") {
                    // `key: value` or `name = value`
                    let k = self.expr()?;
                    let named = if self.eat_op(":") {
                        false
                    } else if self.eat_op("=") {
                        true
                    } else {
                        return self.err("expected `:` in a dictionary");
                    };
                    let v = self.expr()?;
                    // a bare name key (`{x = 1}`) means the string "x"
                    let k = match k {
                        Expr::Ident(n) if named => Expr::Str(n),
                        k => k,
                    };
                    items.push((k, v));
                    if !self.eat_op(",") {
                        break;
                    }
                }
                self.expect_op("}")?;
                Expr::Dict(items)
            }
            Tok::Op(".") => {
                // `.method(args)`: the inherited version
                let name = self.ident()?;
                let args = self.args()?;
                Expr::SuperCall(name, args)
            }
            Tok::Ident(i) => match i.as_str() {
                "null" => Expr::Nil,
                "true" => Expr::Bool(true),
                "false" => Expr::Bool(false),
                "yield" => {
                    if self.is_op("(") {
                        self.skip_parens()?;
                    }
                    Expr::Yield
                }
                _ => Expr::Ident(i),
            },
            t => return self.err(format!("unexpected {t:?} in an expression")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_script_parses() {
        let s = parse(
            "x.gd",
            "extends CharacterState\n\nvar got_hit = false\n\nfunc _frame_0():\n\tgot_hit = false\n\nfunc _frame_1():\n\tif not got_hit:\n\t\thost.has_hyper_armor = true\n\thost.start_projectile_invulnerability()\n",
        )
        .unwrap();
        assert!(matches!(s.extends, Some(Extends::Class(ref c)) if c == "CharacterState"));
        assert_eq!(s.members.len(), 1);
        assert!(s.funcs.contains_key("_frame_1"));
    }

    #[test]
    fn expressions_bind_the_usual_way() {
        let s = parse("x.gd", "func f():\n\treturn 1 + 2 * 3 if a and not b else -x.y[0]\n").unwrap();
        assert!(s.funcs.contains_key("f"));
    }

    #[test]
    fn match_enum_const_and_exports() {
        let src = "extends \"res://a/b.gd\"\nconst K = 5\nenum Mode { A, B = 3 }\nexport (PackedScene) var proj\nexport(int, 0, 10) var n = 2\nonready var node = $\"%Box\"\nsignal hit(who)\nfunc g(x, y = 2):\n\tmatch x:\n\t\t1, 2:\n\t\t\tpass\n\t\t_:\n\t\t\treturn y\n";
        let s = parse("x.gd", src).unwrap();
        assert_eq!(s.consts.len(), 1);
        assert_eq!(s.enums.len(), 1);
        assert_eq!(s.members.len(), 3);
        assert_eq!(s.signals, vec!["hit"]);
    }
}
