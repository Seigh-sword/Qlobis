//! Recursive-descent parser for Qlobis.
//!
//! Grammar (informal):
//!
//! ```text
//! program     := (import | item)*
//! import      := '@' 'module' '.' 'import' '(' STRING ')'
//! item        := attr* ('async'? 'defin' | 'struct' | 'trait' | 'impl')
//! attr        := '@' ident ('.' ident)* '(' attr_arg (',' attr_arg)* ')'
//! func        := 'async'? 'defin' ident generics? '(' params? ')' ('->' type)? block?
//! generics    := '<' ident (',' ident)* '>'
//! params      := param (',' param)* ','?
//! param       := 'self' | ident ':' type
//! struct      := 'struct' ident generics? '{' field* '}'
//! field       := ident ':' type (',' | ';')?
//! trait       := 'trait' ident '{' method_sig* '}'
//! impl        := 'impl' type ['for' type] '{' func* '}'
//! block       := '{' stmt* '}'
//! stmt        := 'let' | 'if' | 'while' | 'for' | 'return' | 'throw' | 'try'
//!              | 'break' | 'continue' | block | expr ['=' expr]
//! type        := 'fn' '(' type,* ')' '->' type | ident ('<' type,* '>')?
//! ```

use qlobis_ast::diag::{suggest_name, Diagnostic, Diagnostics};
use qlobis_ast::*;
use qlobis_lexer::{lex, StrRawPart, Tok, Token};

/// Parse one source file into a [`Program`].
pub fn parse_file(file: FileId, src: &str, diags: &mut Diagnostics) -> Program {
    let lexed = lex(file, src, 0, diags);
    let mut p = Parser::new(file, lexed.tokens, lexed.docs, diags);
    p.parse_program()
}

struct Parser<'a> {
    file: FileId,
    tokens: Vec<Token>,
    pos: usize,
    docs: Vec<(u32, qlobis_lexer::DocComment)>,
    doc_idx: usize,
    diags: &'a mut Diagnostics,
}

type PResult<T> = Result<T, ()>;

impl<'a> Parser<'a> {
    fn new(
        file: FileId,
        tokens: Vec<Token>,
        docs: Vec<(u32, qlobis_lexer::DocComment)>,
        diags: &'a mut Diagnostics,
    ) -> Self {
        Parser { file, tokens, pos: 0, docs, doc_idx: 0, diags }
    }

    // -- token helpers -----------------------------------------------------

    fn peek(&self) -> &Tok {
        &self.tokens[self.pos.min(self.tokens.len() - 1)].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.tokens[(self.pos + n).min(self.tokens.len() - 1)].tok
    }

    fn span(&self) -> Span {
        self.tokens[self.pos.min(self.tokens.len() - 1)].span
    }

    fn prev_span(&self) -> Span {
        self.tokens[self.pos.saturating_sub(1)].span
    }

    fn at(&self, t: &Tok) -> bool {
        std::mem::discriminant(self.peek()) == std::mem::discriminant(t)
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), Tok::Eof)
    }

    fn bump(&mut self) -> Tok {
        let t = self.tokens[self.pos.min(self.tokens.len() - 1)].tok.clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, t: Tok, what: &str) -> PResult<Token> {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(&t) {
            Ok(Token { tok: self.bump(), span: self.prev_span() })
        } else {
            let span = self.span();
            self.diags.push(
                Diagnostic::error(format!("expected {}, found {}", what, self.peek().describe()))
                    .code("E0200")
                    .primary(span, format!("expected {}", what)),
            );
            Err(())
        }
    }

    fn expect_ident(&mut self, what: &str) -> PResult<Ident> {
        let span = self.span();
        match self.peek().clone() {
            Tok::Ident(name) => {
                self.bump();
                Ok(Ident::new(name, span))
            }
            other => {
                self.diags.push(
                    Diagnostic::error(format!("expected {}, found {}", what, other.describe()))
                        .code("E0200")
                        .primary(span, format!("expected {}", what)),
                );
                Err(())
            }
        }
    }

    /// Accept an optional trailing `,` or `;` (or nothing).
    fn eat_separator(&mut self) {
        if matches!(self.peek(), Tok::Comma | Tok::Semi) {
            self.bump();
        }
    }

    fn error(&mut self, msg: impl Into<String>, span: Span) {
        self.diags.push(Diagnostic::error(msg).code("E0201").primary(span, "here"));
    }

    fn error_help(&mut self, msg: impl Into<String>, span: Span, help: String) {
        self.diags.push(
            Diagnostic::error(msg).code("E0201").primary(span, "here").help(help),
        );
    }

    /// Consume doc comments that appear before `offset`.
    fn take_docs_before(&mut self, offset: u32) -> Vec<String> {
        let mut out = Vec::new();
        while self.doc_idx < self.docs.len() && self.docs[self.doc_idx].0 < offset {
            out.push(self.docs[self.doc_idx].1.text.clone());
            self.doc_idx += 1;
        }
        out
    }

    // -- program & items ---------------------------------------------------

    fn parse_program(&mut self) -> Program {
        let mut imports = Vec::new();
        let mut items = Vec::new();
        loop {
            if self.at_eof() {
                break;
            }
            let doc_start = self.span().start;
            let doc = self.take_docs_before(doc_start + 1);
            match self.parse_item_or_import(doc, &mut items) {
                Ok(Some(import)) => imports.push(import),
                Ok(None) => {}
                Err(()) => {
                    if self.at_eof() {
                        break;
                    }
                    self.recover_item();
                }
            }
        }
        Program { imports, items, file: self.file }
    }

    /// Skip tokens until the start of the next likely item.
    fn recover_item(&mut self) {
        let mut depth = 0i32;
        while !self.at_eof() {
            match self.peek() {
                Tok::LBrace => depth += 1,
                Tok::RBrace => {
                    if depth > 0 {
                        depth -= 1;
                    }
                }
                Tok::Defin | Tok::Struct | Tok::Trait | Tok::Impl | Tok::At => {
                    if depth <= 0 {
                        return;
                    }
                }
                _ => {}
            }
            self.bump();
        }
    }

    fn parse_item_or_import(
        &mut self,
        doc: Vec<String>,
        items: &mut Vec<Item>,
    ) -> PResult<Option<Import>> {
        // Attributes (including @module.import and @target).
        let mut target: Option<Vec<Ident>> = None;
        loop {
            if !matches!(self.peek(), Tok::At) {
                break;
            }
            let at_span = self.span();
            self.bump(); // @
            let path = self.parse_attr_path()?;
            let path_str = path
                .iter()
                .map(|i| i.name.as_str())
                .collect::<Vec<_>>()
                .join(".");
            match path_str.as_str() {
                "module.import" => {
                    self.expect(Tok::LParen, "`(` after `@module.import`")?;
                    let (s, span) = self.parse_string_arg("package path")?;
                    self.expect(Tok::RParen, "`)`")?;
                    self.eat_separator();
                    return Ok(Some(Import {
                        path: s,
                        alias: None,
                        doc,
                        span: at_span.merge(span),
                    }));
                }
                "target" => {
                    self.expect(Tok::LParen, "`(` after `@target`")?;
                    let mut names = Vec::new();
                    loop {
                        let sp = self.span();
                        let name = match self.peek().clone() {
                            Tok::Ident(n) => {
                                self.bump();
                                n
                            }
                            other => {
                                let sp = self.span();
                                self.error(
                                    format!(
                                        "expected target name, found {}",
                                        other.describe()
                                    ),
                                    sp,
                                );
                                return Err(());
                            }
                        };
                        let valid = ["web", "python", "php", "kotlin", "native"];
                        if !valid.contains(&name.as_str()) {
                            self.error_help(
                                format!("unknown target `{}`", name),
                                sp,
                                format!(
                                    "valid targets are: {}",
                                    valid.join(", ")
                                ),
                            );
                            return Err(());
                        }
                        names.push(Ident::new(name, sp));
                        if matches!(self.peek(), Tok::Comma) {
                            self.bump();
                            continue;
                        }
                        break;
                    }
                    self.expect(Tok::RParen, "`)`")?;
                    if target.is_some() {
                        self.error("duplicate `@target` attribute", at_span);
                    }
                    target = Some(names);
                }
                _ => {
                    let candidates = ["module.import", "target"];
                    let help = suggest_name(&path_str, candidates.iter().copied())
                        .map(|c| format!("did you mean `@{}`?", c))
                        .unwrap_or_else(|| {
                            "supported attributes are `@module.import(...)` and `@target(...)`"
                                .to_string()
                        });
                    self.error_help(
                        format!("unknown attribute `@{}`", path_str),
                        at_span.merge(self.prev_span()),
                        help,
                    );
                    return Err(());
                }
            }
        }

        // The item itself.
        let item = self.parse_item(target, doc)?;
        items.push(item);
        Ok(None)
    }

    fn parse_attr_path(&mut self) -> PResult<Vec<Ident>> {
        let mut path = vec![self.expect_ident("attribute name")?];
        while matches!(self.peek(), Tok::Dot) {
            self.bump();
            path.push(self.expect_ident("attribute name")?);
        }
        Ok(path)
    }

    /// Parse a single string literal argument (without interpolation).
    fn parse_string_arg(&mut self, what: &str) -> PResult<(String, Span)> {
        let span = self.span();
        match self.peek().clone() {
            Tok::Str(parts) => {
                self.bump();
                let mut out = String::new();
                for p in &parts {
                    match p {
                        StrRawPart::Lit(s) => out.push_str(s),
                        StrRawPart::Interp { .. } => {
                            self.error(
                                format!("string interpolation is not allowed in {}", what),
                                span,
                            );
                            return Err(());
                        }
                    }
                }
                Ok((out, span))
            }
            other => {
                self.error(
                    format!("expected string for {}, found {}", what, other.describe()),
                    span,
                );
                Err(())
            }
        }
    }

    fn parse_item(&mut self, target: Option<Vec<Ident>>, doc: Vec<String>) -> PResult<Item> {
        let start = self.span();
        let is_async = matches!(self.peek(), Tok::Async);
        if is_async {
            self.bump();
        }
        match self.peek().clone() {
            Tok::Defin => Ok(Item::Func(self.parse_func(target, doc, is_async)?)),
            Tok::Struct if !is_async => Ok(Item::Struct(self.parse_struct(doc)?)),
            Tok::Trait if !is_async => Ok(Item::Trait(self.parse_trait(doc)?)),
            Tok::Impl if !is_async => Ok(Item::Impl(self.parse_impl()?)),
            other => {
                let msg = format!("expected an item, found {}", other.describe());
                self.error(msg, start);
                Err(())
            }
        }
    }

    fn parse_generics(&mut self) -> PResult<Vec<Ident>> {
        let mut out = Vec::new();
        if matches!(self.peek(), Tok::Lt) {
            self.bump();
            loop {
                out.push(self.expect_ident("type parameter name")?);
                if matches!(self.peek(), Tok::Comma) {
                    self.bump();
                    continue;
                }
                break;
            }
            self.expect_gt("`>`")?;
        }
        Ok(out)
    }

    fn parse_func(
        &mut self,
        target: Option<Vec<Ident>>,
        doc: Vec<String>,
        is_async: bool,
    ) -> PResult<FuncDef> {
        let start = self.span();
        self.expect(Tok::Defin, "`defin`")?;
        let name = self.expect_ident("function name")?;
        let generics = self.parse_generics()?;
        self.expect(Tok::LParen, "`(` after function name")?;
        let params = self.parse_params()?;
        self.expect(Tok::RParen, "`)`")?;
        let ret = if matches!(self.peek(), Tok::Arrow) {
            self.bump();
            Some(self.parse_type()?)
        } else {
            None
        };
        let body = if matches!(self.peek(), Tok::LBrace) {
            Some(self.parse_block()?)
        } else {
            None
        };
        if body.is_none() && target.is_some() {
            // Allowed only inside traits (checked by semantic analysis).
        }
        Ok(FuncDef {
            name,
            generics,
            params,
            ret,
            body,
            target,
            is_async,
            doc,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_params(&mut self) -> PResult<Vec<Param>> {
        let mut params = Vec::new();
        if matches!(self.peek(), Tok::RParen) {
            return Ok(params);
        }
        loop {
            let start = self.span();
            if matches!(self.peek(), Tok::SelfVal) {
                self.bump();
                params.push(Param {
                    name: Ident::new("self", start),
                    ty: None,
                    is_self: true,
                    span: start.merge(self.prev_span()),
                });
            } else {
                let name = self.expect_ident("parameter name")?;
                self.expect(Tok::Colon, "`:` after parameter name")?;
                let ty = self.parse_type()?;
                let span = start.merge(self.prev_span());
                params.push(Param { name, ty: Some(ty), is_self: false, span });
            }
            if matches!(self.peek(), Tok::Comma) {
                self.bump();
                if matches!(self.peek(), Tok::RParen) {
                    break;
                }
                continue;
            }
            break;
        }
        Ok(params)
    }

    fn parse_struct(&mut self, doc: Vec<String>) -> PResult<StructDef> {
        let start = self.span();
        self.expect(Tok::Struct, "`struct`")?;
        let name = self.expect_ident("struct name")?;
        let generics = self.parse_generics()?;
        self.expect(Tok::LBrace, "`{`")?;
        let mut fields = Vec::new();
        loop {
            if matches!(self.peek(), Tok::RBrace) {
                break;
            }
            if self.at_eof() {
                self.error("unterminated struct body", self.span());
                return Err(());
            }
            let fstart = self.span();
            let fdoc = self.take_docs_before(fstart.start + 1);
            let fname = self.expect_ident("field name")?;
            self.expect(Tok::Colon, "`:` after field name")?;
            let ty = self.parse_type()?;
            fields.push(Field {
                name: fname,
                ty,
                doc: fdoc,
                span: fstart.merge(self.prev_span()),
            });
            self.eat_separator();
        }
        self.expect(Tok::RBrace, "`}`")?;
        Ok(StructDef {
            name,
            generics,
            fields,
            doc,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_trait(&mut self, doc: Vec<String>) -> PResult<TraitDef> {
        let start = self.span();
        self.expect(Tok::Trait, "`trait`")?;
        let name = self.expect_ident("trait name")?;
        self.expect(Tok::LBrace, "`{`")?;
        let mut methods = Vec::new();
        loop {
            if matches!(self.peek(), Tok::RBrace) {
                break;
            }
            if self.at_eof() {
                self.error("unterminated trait body", self.span());
                return Err(());
            }
            let mstart = self.span();
            let mdoc = self.take_docs_before(mstart.start + 1);
            let is_async = matches!(self.peek(), Tok::Async);
            if is_async {
                self.bump();
            }
            let method = self.parse_func(None, mdoc, is_async)?;
            methods.push(method);
            self.eat_separator();
        }
        self.expect(Tok::RBrace, "`}`")?;
        Ok(TraitDef {
            name,
            methods,
            doc,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_impl(&mut self) -> PResult<ImplDef> {
        let start = self.span();
        self.expect(Tok::Impl, "`impl`")?;
        let first = self.parse_type()?;
        let (trait_name, self_ty) = if matches!(self.peek(), Tok::For) {
            self.bump();
            let tname = match &first {
                TypeExpr::Named { name, args, span } if args.is_empty() => {
                    Ident::new(name.clone(), *span)
                }
                other => {
                    self.error(
                        format!(
                            "expected a trait name before `for`, found `{}`",
                            type_expr_to_string(other)
                        ),
                        other.span(),
                    );
                    return Err(());
                }
            };
            (Some(tname), self.parse_type()?)
        } else {
            (None, first)
        };
        self.expect(Tok::LBrace, "`{`")?;
        let mut methods = Vec::new();
        loop {
            if matches!(self.peek(), Tok::RBrace) {
                break;
            }
            if self.at_eof() {
                self.error("unterminated impl block", self.span());
                return Err(());
            }
            let mstart = self.span();
            let mdoc = self.take_docs_before(mstart.start + 1);
            let is_async = matches!(self.peek(), Tok::Async);
            if is_async {
                self.bump();
            }
            methods.push(self.parse_func(None, mdoc, is_async)?);
            self.eat_separator();
        }
        self.expect(Tok::RBrace, "`}`")?;
        Ok(ImplDef {
            trait_name,
            self_ty,
            methods,
            span: start.merge(self.prev_span()),
        })
    }

    // -- types -------------------------------------------------------------

    /// Consume a `>` in type position. A `>>` token is split into two `>`
    /// tokens so `list<list<T>>` parses like it reads.
    fn expect_gt(&mut self, what: &str) -> PResult<()> {
        if matches!(self.peek(), Tok::Shr) {
            let sp = self.tokens[self.pos].span;
            self.tokens.splice(
                self.pos..self.pos + 1,
                [
                    qlobis_lexer::Token {
                        tok: Tok::Gt,
                        span: sp,
                    },
                    qlobis_lexer::Token {
                        tok: Tok::Gt,
                        span: sp,
                    },
                ],
            );
        }
        self.expect(Tok::Gt, what)?;
        Ok(())
    }

    fn parse_type(&mut self) -> PResult<TypeExpr> {
        let start = self.span();
        match self.peek().clone() {
            Tok::Ident(name) if name == "fn" && matches!(self.peek_at(1), Tok::LParen) => {
                self.bump();
                self.bump();
                let mut params = Vec::new();
                if !matches!(self.peek(), Tok::RParen) {
                    loop {
                        params.push(self.parse_type()?);
                        if matches!(self.peek(), Tok::Comma) {
                            self.bump();
                            continue;
                        }
                        break;
                    }
                }
                self.expect(Tok::RParen, "`)`")?;
                self.expect(Tok::Arrow, "`->` in function type")?;
                let ret = self.parse_type()?;
                let span = start.merge(self.prev_span());
                Ok(TypeExpr::Fn { params, ret: Box::new(ret), span })
            }
            Tok::Ident(name) => {
                self.bump();
                let mut args = Vec::new();
                if matches!(self.peek(), Tok::Lt) {
                    self.bump();
                    loop {
                        args.push(self.parse_type()?);
                        if matches!(self.peek(), Tok::Comma) {
                            self.bump();
                            continue;
                        }
                        break;
                    }
                    self.expect_gt("`>`")?;
                }
                let span = start.merge(self.prev_span());
                Ok(TypeExpr::Named { name, args, span })
            }
            other => {
                let msg = format!("expected a type, found {}", other.describe());
                self.error(msg, start);
                Err(())
            }
        }
    }

    // -- statements --------------------------------------------------------

    fn parse_block(&mut self) -> PResult<Block> {
        let start = self.span();
        self.expect(Tok::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        loop {
            match self.peek() {
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Eof => {
                    self.error("unterminated block", self.span());
                    return Err(());
                }
                _ => {
                    stmts.push(self.parse_stmt()?);
                    while matches!(self.peek(), Tok::Semi) {
                        self.bump();
                    }
                }
            }
        }
        Ok(Block { stmts, span: start.merge(self.prev_span()) })
    }

    fn parse_stmt(&mut self) -> PResult<Stmt> {
        let start = self.span();
        match self.peek() {
            Tok::Let => {
                self.bump();
                let name = self.expect_ident("variable name")?;
                let ty = if matches!(self.peek(), Tok::Colon) {
                    self.bump();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                self.expect(Tok::Assign, "`=` in let binding")?;
                let value = self.parse_expr()?;
                Ok(Stmt::Let {
                    name,
                    ty,
                    value,
                    span: start.merge(self.prev_span()),
                })
            }
            Tok::If => self.parse_if(),
            Tok::While => {
                self.bump();
                let cond = self.parse_expr()?;
                let body = self.parse_block()?;
                Ok(Stmt::While {
                    cond,
                    body,
                    span: start.merge(self.prev_span()),
                })
            }
            Tok::For => {
                self.bump();
                let var = self.expect_ident("loop variable")?;
                self.expect(Tok::In, "`in` after loop variable")?;
                let iter = self.parse_expr()?;
                let body = self.parse_block()?;
                Ok(Stmt::For {
                    var,
                    iter,
                    body,
                    span: start.merge(self.prev_span()),
                })
            }
            Tok::Return => {
                self.bump();
                let value = if self.starts_expr() {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                Ok(Stmt::Return { value, span: start.merge(self.prev_span()) })
            }
            Tok::Throw => {
                self.bump();
                let value = self.parse_expr()?;
                Ok(Stmt::Throw { value, span: start.merge(self.prev_span()) })
            }
            Tok::Try => {
                self.bump();
                let body = self.parse_block()?;
                // `throw name { ... }` — catch clause.
                let catch = if matches!(self.peek(), Tok::Throw)
                    && matches!(self.peek_at(1), Tok::Ident(_))
                    && matches!(self.peek_at(2), Tok::LBrace)
                {
                    let cstart = self.span();
                    self.bump(); // throw
                    let name = self.expect_ident("catch binding")?;
                    let cbody = self.parse_block()?;
                    Some(Catch {
                        name,
                        body: cbody,
                        span: cstart.merge(self.prev_span()),
                    })
                } else {
                    None
                };
                Ok(Stmt::Try {
                    body,
                    catch,
                    span: start.merge(self.prev_span()),
                })
            }
            Tok::Break => {
                self.bump();
                Ok(Stmt::Break { span: start.merge(self.prev_span()) })
            }
            Tok::Continue => {
                self.bump();
                Ok(Stmt::Continue { span: start.merge(self.prev_span()) })
            }
            Tok::LBrace => {
                let b = self.parse_block()?;
                Ok(Stmt::Block(b))
            }
            _ => {
                let expr = self.parse_expr()?;
                if self.at(&Tok::Assign) {
                    self.bump();
                    let value = self.parse_expr()?;
                    if !expr.is_lvalue() {
                        self.error_help(
                            "invalid assignment target",
                            expr.span(),
                            "only variables, fields (`x.y`) and index expressions (`x[i]`) can be assigned"
                                .to_string(),
                        );
                    }
                    Ok(Stmt::Assign {
                        target: expr,
                        op: None,
                        value,
                        span: start.merge(self.prev_span()),
                    })
                } else {
                    let op = match self.peek() {
                        Tok::PlusAssign => Some(BinOp::Add),
                        Tok::MinusAssign => Some(BinOp::Sub),
                        Tok::StarAssign => Some(BinOp::Mul),
                        Tok::SlashAssign => Some(BinOp::Div),
                        Tok::PercentAssign => Some(BinOp::Mod),
                        _ => None,
                    };
                    if let Some(op) = op {
                        self.bump();
                        let value = self.parse_expr()?;
                        if !expr.is_lvalue() {
                            self.error("invalid assignment target", expr.span());
                        }
                        Ok(Stmt::Assign {
                            target: expr,
                            op: Some(op),
                            value,
                            span: start.merge(self.prev_span()),
                        })
                    } else {
                        Ok(Stmt::Expr { expr, span: start.merge(self.prev_span()) })
                    }
                }
            }
        }
    }

    fn parse_if(&mut self) -> PResult<Stmt> {
        let start = self.span();
        self.expect(Tok::If, "`if`")?;
        let cond = self.parse_expr()?;
        let then = self.parse_block()?;
        let otherwise = if matches!(self.peek(), Tok::Else) {
            self.bump();
            if matches!(self.peek(), Tok::If) {
                let nested = self.parse_if()?;
                let span = nested.span();
                Some(Block {
                    stmts: vec![nested],
                    span,
                })
            } else {
                Some(self.parse_block()?)
            }
        } else {
            None
        };
        Ok(Stmt::If {
            cond,
            then,
            otherwise,
            span: start.merge(self.prev_span()),
        })
    }

    // -- expressions -------------------------------------------------------

    fn starts_expr(&self) -> bool {
        matches!(
            self.peek(),
            Tok::Int(_)
                | Tok::Float(_)
                | Tok::Str(_)
                | Tok::Ident(_)
                | Tok::True
                | Tok::False
                | Tok::None
                | Tok::SelfVal
                | Tok::LParen
                | Tok::LBracket
                | Tok::LBrace
                | Tok::Minus
                | Tok::Not
                | Tok::Bang
                | Tok::Tilde
                | Tok::Await
        )
    }

    pub fn parse_expr(&mut self) -> PResult<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_and()?;
        while matches!(self.peek(), Tok::Or) {
            self.bump();
            let rhs = self.parse_and()?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op: BinOp::Or,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_cmp()?;
        while matches!(self.peek(), Tok::And) {
            self.bump();
            let rhs = self.parse_cmp()?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op: BinOp::And,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_cmp(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_bitor()?;
        loop {
            let op = match self.peek() {
                Tok::Eq => BinOp::Eq,
                Tok::Ne => BinOp::Ne,
                Tok::Lt => BinOp::Lt,
                Tok::Le => BinOp::Le,
                Tok::Gt => BinOp::Gt,
                Tok::Ge => BinOp::Ge,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_bitor()?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_bitor(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_bitxor()?;
        while matches!(self.peek(), Tok::Pipe) {
            self.bump();
            let rhs = self.parse_bitxor()?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op: BinOp::BitOr,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_bitxor(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_bitand()?;
        while matches!(self.peek(), Tok::Caret) {
            self.bump();
            let rhs = self.parse_bitand()?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op: BinOp::BitXor,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_bitand(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_shift()?;
        while matches!(self.peek(), Tok::Amp) {
            self.bump();
            let rhs = self.parse_shift()?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op: BinOp::BitAnd,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_shift(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_term()?;
        loop {
            let op = match self.peek() {
                Tok::Shl => BinOp::Shl,
                Tok::Shr => BinOp::Shr,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_term()?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_term(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_factor()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_factor()?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_factor(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Mod,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_unary()?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        let start = self.span();
        match self.peek() {
            Tok::Minus => {
                self.bump();
                let expr = self.parse_unary()?;
                let span = start.merge(expr.span());
                Ok(Expr::Unary { op: UnOp::Neg, expr: Box::new(expr), span })
            }
            Tok::Not | Tok::Bang => {
                self.bump();
                let expr = self.parse_unary()?;
                let span = start.merge(expr.span());
                Ok(Expr::Unary { op: UnOp::Not, expr: Box::new(expr), span })
            }
            Tok::Tilde => {
                self.bump();
                let expr = self.parse_unary()?;
                let span = start.merge(expr.span());
                Ok(Expr::Unary { op: UnOp::BitNot, expr: Box::new(expr), span })
            }
            Tok::Await => {
                self.bump();
                let expr = self.parse_unary()?;
                let span = start.merge(expr.span());
                Ok(Expr::Await { expr: Box::new(expr), span })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek() {
                Tok::LParen => {
                    self.bump();
                    let args = self.parse_args()?;
                    let span = expr.span().merge(self.prev_span());
                    expr = Expr::Call { func: Box::new(expr), args, span };
                }
                Tok::LBracket => {
                    self.bump();
                    let index = self.parse_expr()?;
                    self.expect(Tok::RBracket, "`]`")?;
                    let span = expr.span().merge(self.prev_span());
                    expr = Expr::Index {
                        recv: Box::new(expr),
                        index: Box::new(index),
                        span,
                    };
                }
                Tok::Dot => {
                    self.bump();
                    let name = self.expect_ident("member name")?;
                    if matches!(self.peek(), Tok::LParen) {
                        self.bump();
                        let args = self.parse_args()?;
                        let span = expr.span().merge(self.prev_span());
                        expr = Expr::Method {
                            recv: Box::new(expr),
                            name,
                            args,
                            span,
                        };
                    } else {
                        let span = expr.span().merge(name.span);
                        expr = Expr::Member {
                            recv: Box::new(expr),
                            name,
                            span,
                        };
                    }
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_args(&mut self) -> PResult<Vec<Expr>> {
        let mut args = Vec::new();
        if matches!(self.peek(), Tok::RParen) {
            self.bump();
            return Ok(args);
        }
        loop {
            args.push(self.parse_expr()?);
            if matches!(self.peek(), Tok::Comma) {
                self.bump();
                if matches!(self.peek(), Tok::RParen) {
                    break;
                }
                continue;
            }
            break;
        }
        self.expect(Tok::RParen, "`)`")?;
        Ok(args)
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let start = self.span();
        match self.peek().clone() {
            Tok::Int(v) => {
                self.bump();
                Ok(Expr::Int(v, start))
            }
            Tok::Float(v) => {
                self.bump();
                Ok(Expr::Float(v, start))
            }
            Tok::True => {
                self.bump();
                Ok(Expr::Bool(true, start))
            }
            Tok::False => {
                self.bump();
                Ok(Expr::Bool(false, start))
            }
            Tok::None => {
                self.bump();
                Ok(Expr::None(start))
            }
            Tok::SelfVal => {
                self.bump();
                Ok(Expr::Ident(Ident::new("self", start)))
            }
            Tok::Ident(name) => {
                self.bump();
                Ok(Expr::Ident(Ident::new(name, start)))
            }
            Tok::Str(parts) => {
                self.bump();
                let str_parts = self.resolve_string_parts(&parts)?;
                Ok(Expr::Str(str_parts, start.merge(self.prev_span())))
            }
            Tok::LParen => {
                self.bump();
                let inner = self.parse_expr()?;
                self.expect(Tok::RParen, "`)`")?;
                Ok(inner)
            }
            Tok::LBracket => {
                self.bump();
                let mut items = Vec::new();
                if !matches!(self.peek(), Tok::RBracket) {
                    loop {
                        items.push(self.parse_expr()?);
                        if matches!(self.peek(), Tok::Comma) {
                            self.bump();
                            if matches!(self.peek(), Tok::RBracket) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect(Tok::RBracket, "`]`")?;
                Ok(Expr::List(items, start.merge(self.prev_span())))
            }
            Tok::LBrace => {
                self.bump();
                let mut entries = Vec::new();
                if !matches!(self.peek(), Tok::RBrace) {
                    loop {
                        let k = self.parse_expr()?;
                        self.expect(Tok::Colon, "`:` between map key and value")?;
                        let v = self.parse_expr()?;
                        entries.push((k, v));
                        if matches!(self.peek(), Tok::Comma) {
                            self.bump();
                            if matches!(self.peek(), Tok::RBrace) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect(Tok::RBrace, "`}`")?;
                Ok(Expr::Map(entries, start.merge(self.prev_span())))
            }
            other => {
                let msg = format!("expected an expression, found {}", other.describe());
                self.error(msg, start);
                Err(())
            }
        }
    }

    /// Turn raw string parts (with `{expr}` segments) into typed [`StrPart`]s,
    /// recursively parsing each interpolation.
    fn resolve_string_parts(&mut self, parts: &[StrRawPart]) -> PResult<Vec<StrPart>> {
        let mut out = Vec::new();
        for p in parts {
            match p {
                StrRawPart::Lit(s) => out.push(StrPart::Lit(s.clone())),
                StrRawPart::Interp { src, span } => {
                    let expr = parse_expr_from(self.file, src, *span, &mut *self.diags)?;
                    out.push(StrPart::Interp(expr));
                }
            }
        }
        Ok(out)
    }
}

/// Parse a standalone expression from `src` (used for string interpolation).
/// Spans are shifted by `span.start` so they point into the enclosing file.
fn parse_expr_from(
    file: FileId,
    src: &str,
    span: Span,
    diags: &mut Diagnostics,
) -> PResult<Expr> {
    let lexed = lex(file, src, span.start, diags);
    let mut sub = Parser::new(file, lexed.tokens, lexed.docs, diags);
    let expr = sub.parse_expr()?;
    if !sub.at_eof() {
        let sp = sub.span();
        sub.error("unexpected trailing tokens in interpolation", sp);
        return Err(());
    }
    Ok(expr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> Program {
        let mut d = Diagnostics::new();
        let p = parse_file(0, src, &mut d);
        if d.has_errors() {
            panic!("parse errors: {}", d.render(&{
                let mut m = SourceMap::new();
                m.add_file("t.qbs", src);
                m
            }, false));
        }
        p
    }

    fn parse_err(src: &str) -> Diagnostics {
        let mut d = Diagnostics::new();
        let _ = parse_file(0, src, &mut d);
        assert!(d.has_errors(), "expected parse errors for {:?}", src);
        d
    }

    #[test]
    fn parse_main_and_imports() {
        let p = parse_ok("@module.import(\"std::io\")\n\ndefin main() {\n  io.println(\"hi\")\n}\n");
        assert_eq!(p.imports.len(), 1);
        assert_eq!(p.imports[0].path, "std::io");
        assert_eq!(p.items.len(), 1);
        match &p.items[0] {
            Item::Func(f) => {
                assert_eq!(f.name.name, "main");
                assert!(f.ret.is_none());
                assert_eq!(f.body.as_ref().unwrap().stmts.len(), 1);
            }
            other => panic!("expected func, {:?}", other),
        }
    }

    #[test]
    fn parse_typed_function_and_generics() {
        let p = parse_ok("defin id<T>(x: T, n: int) -> T {\n  return x\n}\n");
        match &p.items[0] {
            Item::Func(f) => {
                assert_eq!(f.generics.len(), 1);
                assert_eq!(f.generics[0].name, "T");
                assert_eq!(f.params.len(), 2);
                assert!(f.params[0].ty.is_some());
                assert_eq!(type_expr_to_string(f.ret.as_ref().unwrap()), "T");
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn parse_target_hatch() {
        let p = parse_ok("@target(web)\ndefin fetch(url: str) -> str {\n  return url\n}\n");
        match &p.items[0] {
            Item::Func(f) => {
                let t = f.target.as_ref().expect("target set");
                assert_eq!(t[0].name, "web");
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn parse_struct_trait_impl() {
        let p = parse_ok(
            "struct Point {\n  x: int\n  y: int\n}\n\
             trait Shape {\n  defin area(self) -> float\n}\n\
             impl Shape for Point {\n  defin area(self) -> float {\n    return float(self.x)\n  }\n}\n\
             impl Point {\n  defin sum(self) -> int {\n    return self.x + self.y\n  }\n}\n",
        );
        assert_eq!(p.items.len(), 4);
        assert!(matches!(&p.items[0], Item::Struct(s) if s.name.name == "Point" && s.fields.len() == 2));
        assert!(matches!(&p.items[1], Item::Trait(t) if t.name.name == "Shape" && t.methods.len() == 1));
        match &p.items[2] {
            Item::Impl(i) => {
                assert_eq!(i.trait_name.as_ref().unwrap().name, "Shape");
                assert_eq!(i.methods.len(), 1);
            }
            other => panic!("{:?}", other),
        }
        assert!(matches!(&p.items[3], Item::Impl(i) if i.trait_name.is_none()));
    }

    #[test]
    fn parse_control_flow_and_try() {
        let p = parse_ok(
            "defin main() {\n  let xs = [1, 2, 3]\n  for x in xs {\n    if x > 1 {\n      continue\n    }\n  }\n  \
             while true {\n    break\n  }\n  try {\n    throw \"bad\"\n  } throw err {\n    let e = err\n  }\n}\n",
        );
        match &p.items[0] {
            Item::Func(f) => {
                let stmts = &f.body.as_ref().unwrap().stmts;
                assert_eq!(stmts.len(), 4);
                assert!(matches!(&stmts[3], Stmt::Try { catch: Some(_), .. }));
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn parse_interpolation() {
        let p = parse_ok("defin main() {\n  let name = \"qlobis\"\n  let msg = \"hello {name}!\"\n}\n");
        match &p.items[0] {
            Item::Func(f) => match &f.body.as_ref().unwrap().stmts[1] {
                Stmt::Let { value: Expr::Str(parts, ..), .. } => {
                    assert_eq!(parts.len(), 3);
                    assert!(matches!(&parts[1], StrPart::Interp(Expr::Ident(i)) if i.name == "name"));
                }
                other => panic!("{:?}", other),
            },
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn parse_expression_precedence() {
        let p = parse_ok("defin main() {\n  let x = 1 + 2 * 3 == 7 and 1 < 2 or not false\n}\n");
        match &p.items[0] {
            Item::Func(f) => match &f.body.as_ref().unwrap().stmts[0] {
                Stmt::Let { value, .. } => {
                    // or( and( ==( +(1, *(2,3)), 7), <(1,2)), not(false) )
                    assert!(matches!(value, Expr::Binary { op: BinOp::Or, .. }));
                }
                other => panic!("{:?}", other),
            },
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn parse_map_and_calls() {
        let p = parse_ok(
            "defin main() {\n  let m = {\"a\": 1, \"b\": 2}\n  let v = m[\"a\"]\n  let s = trim(v)\n}\n",
        );
        match &p.items[0] {
            Item::Func(f) => {
                let stmts = &f.body.as_ref().unwrap().stmts;
                assert!(matches!(&stmts[0], Stmt::Let { value: Expr::Map(e, ..), .. } if e.len() == 2));
                assert!(matches!(&stmts[1], Stmt::Let { value: Expr::Index { .. }, .. }));
                assert!(matches!(&stmts[2], Stmt::Let { value: Expr::Call { .. }, .. }));
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn error_unknown_attribute() {
        let d = parse_err("@module.importx(\"x\")\ndefin main() {}\n");
        let mut m = SourceMap::new();
        m.add_file("t.qbs", "");
        let out = d.render(&m, false);
        assert!(out.contains("unknown attribute"));
    }

    #[test]
    fn error_missing_brace_reports() {
        parse_err("defin main() {\n");
    }

    #[test]
    fn docs_attach_to_items() {
        let p = parse_ok("/// Adds numbers.\ndefin add(a: int, b: int) -> int {\n  return a + b\n}\n");
        match &p.items[0] {
            Item::Func(f) => assert_eq!(f.doc, vec!["Adds numbers.".to_string()]),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn fn_type_syntax() {
        let p = parse_ok("defin apply(f: fn(int, int) -> int) -> int {\n  return f(1, 2)\n}\n");
        match &p.items[0] {
            Item::Func(f) => match f.params[0].ty.as_ref().unwrap() {
                TypeExpr::Fn { params, ret, .. } => {
                    assert_eq!(params.len(), 2);
                    assert_eq!(type_expr_to_string(ret), "int");
                }
                other => panic!("{:?}", other),
            },
            other => panic!("{:?}", other),
        }
    }
}
