//! Parser for the bas language

use crate::ast::*;
use crate::span::Span;
use crate::token::{Token, TokenKind};

/// Parse error
#[derive(Clone, Debug)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
    pub expected: Vec<String>,
}

impl ParseError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        ParseError {
            message: message.into(),
            span,
            expected: vec![],
        }
    }
    
    pub fn with_expected(mut self, expected: impl Into<String>) -> Self {
        self.expected.push(expected.into());
        self
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "parse error: {} @ {}", self.message, self.span)?;
        if !self.expected.is_empty() {
            write!(f, " (expected {})", self.expected.join(", "))?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseError {}

/// Parse result
pub type ParseResult<T> = Result<T, ParseError>;

/// The parser
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// When > 0, struct literals are disallowed (e.g. in if/while/match
    /// conditions, where `{` begins the body block instead)
    no_struct_literal: u32,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        // Comments are not significant to the grammar, so drop them up front.
        let tokens: Vec<Token> = tokens
            .into_iter()
            .filter(|t| !matches!(t.value, TokenKind::Comment(_)))
            .collect();
        Parser { tokens, pos: 0, no_struct_literal: 0 }
    }
    
    /// Get the current token without consuming
    pub fn peek(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }
    
    /// Get the token at an offset
    pub fn peek_at(&self, offset: usize) -> &Token {
        let idx = (self.pos + offset).min(self.tokens.len() - 1);
        &self.tokens[idx]
    }
    
    /// Get the current token kind
    pub fn current_kind(&self) -> &TokenKind {
        &self.peek().value
    }
    
    /// Check if current token matches a kind
    pub fn check(&self, kind: &TokenKind) -> bool {
        self.current_kind() == kind
    }
    
    /// Check if current token matches any of the kinds
    pub fn check_any(&self, kinds: &[TokenKind]) -> bool {
        kinds.contains(self.current_kind())
    }
    
    /// Check if current token is a keyword
    pub fn check_keyword(&self, keyword: &str) -> bool {
        match self.current_kind() {
            // Soft/contextual keywords are lexed as identifiers
            TokenKind::Ident(s) => s == keyword,
            // Reserved keywords are lexed as keyword tokens
            _ => self.current_kind().keyword_str() == Some(keyword),
        }
    }
    
    /// Consume the current token and return it
    pub fn bump(&mut self) -> Token {
        let token = self.tokens[self.pos.min(self.tokens.len() - 1)].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        token
    }
    
    /// Consume token if it matches, return Some(token) if consumed
    pub fn eat(&mut self, kind: &TokenKind) -> Option<Token> {
        if self.check(kind) {
            Some(self.bump())
        } else {
            None
        }
    }
    
    /// Expect a specific token, error if not found
    pub fn expect(&mut self, kind: &TokenKind) -> ParseResult<Token> {
        if self.check(kind) {
            Ok(self.bump())
        } else {
            let span = self.peek().span;
            Err(ParseError::new(
                format!("expected {}, found {}", kind, self.current_kind()),
                span,
            ).with_expected(format!("{}", kind)))
        }
    }
    
    /// Expect a specific keyword
    pub fn expect_keyword(&mut self, keyword: &str) -> ParseResult<Token> {
        if self.check_keyword(keyword) {
            Ok(self.bump())
        } else {
            let span = self.peek().span;
            Err(ParseError::new(
                format!("expected keyword '{}', found {}", keyword, self.current_kind()),
                span,
            ))
        }
    }
    
    /// Check if at end of input
    pub fn at_end(&self) -> bool {
        matches!(self.current_kind(), TokenKind::Eof)
    }
    
    /// Parse a complete source file
    pub fn parse_file(&mut self) -> ParseResult<SourceFile> {
        let mut items = Vec::new();
        while !self.at_end() {
            // Skip stray semicolons
            while self.eat(&TokenKind::Semi).is_some() {}
            if self.at_end() { break; }
            
            let item = self.parse_item()?;
            items.push(item);
            
            // Items may be separated by semicolons (optional)
            self.eat(&TokenKind::Semi);
        }
        Ok(SourceFile::new(items))
    }
    
    /// Parse a top-level item
    pub fn parse_item(&mut self) -> ParseResult<Item> {
        // Handle attributes
        let mut attrs = Vec::new();
        while self.check(&TokenKind::Pound) {
            let attr = self.parse_attribute()?;
            attrs.push(attr);
        }
        
        // Handle visibility
        let _visibility = if self.check_keyword("pub") {
            self.parse_visibility()?
        } else {
            Visibility::Private
        };
        
        // Handle unsafe
        let is_unsafe = if self.check_keyword("unsafe") {
            self.bump();
            true
        } else {
            false
        };
        
        // Handle extern
        if self.check_keyword("extern") {
            return self.parse_extern_block(is_unsafe, attrs);
        }
        
        // Dispatch on keyword
        match self.current_kind() {
            TokenKind::KwFn => self.parse_fn(attrs, is_unsafe, false),
            TokenKind::KwStruct => self.parse_struct(attrs),
            TokenKind::KwEnum => self.parse_enum(attrs),
            TokenKind::KwUnion => self.parse_union(attrs),
            TokenKind::KwType => self.parse_type_alias(attrs),
            TokenKind::KwTrait => self.parse_trait(attrs, is_unsafe),
            TokenKind::KwImpl => self.parse_impl(attrs, is_unsafe),
            TokenKind::KwMod => self.parse_mod(attrs),
            TokenKind::KwUse => self.parse_use(attrs),
            TokenKind::KwConst => self.parse_const(attrs, false),
            TokenKind::KwStatic => self.parse_const(attrs, true),
            TokenKind::Ident(_) if self.check_keyword("macro") => self.parse_macro_def(attrs),
            _ => {
                // Try to parse as macro invocation (e.g., macro_rules!)
                if let TokenKind::Ident(name) = self.current_kind().clone() {
                    if name == "macro_rules" {
                        return self.parse_macro_rules(attrs);
                    }
                }
                let span = self.peek().span;
                Err(ParseError::new(
                    format!("expected item, found {}", self.current_kind()),
                    span,
                ))
            }
        }
    }
    
    /// Parse attributes: #[...] or #![...]
    fn parse_attribute(&mut self) -> ParseResult<Attribute> {
        let start = self.peek().span;
        self.expect(&TokenKind::Pound)?;
        let is_inner = if self.check(&TokenKind::Not) {
            self.bump();
            true
        } else {
            false
        };
        self.expect(&TokenKind::LBracket)?;
        
        // Parse path
        let path = self.parse_path()?;
        
        // Parse optional args
        let mut args = Vec::new();
        if self.check(&TokenKind::LParen) {
            self.bump();
            while !self.check(&TokenKind::RParen) && !self.at_end() {
                let expr = self.parse_expr()?;
                args.push(expr);
                if !self.eat(&TokenKind::Comma).is_some() {
                    break;
                }
            }
            self.expect(&TokenKind::RParen)?;
        }
        
        self.expect(&TokenKind::RBracket)?;
        let _span = start.merge(self.peek().span);
        Ok(Attribute { path, args, is_inner, ..Default::default() })
    }
    
    /// Parse visibility
    fn parse_visibility(&mut self) -> ParseResult<Visibility> {
        self.expect_keyword("pub")?;
        if self.check(&TokenKind::LParen) {
            self.bump();
            let vis = if self.check_keyword("crate") {
                self.bump();
                Visibility::PubCrate
            } else if self.check_keyword("super") {
                self.bump();
                Visibility::PubSuper
            } else if self.check_keyword("in") {
                self.bump();
                let path = self.parse_path()?;
                Visibility::PubIn(path)
            } else {
                let span = self.peek().span;
                return Err(ParseError::new("expected crate, super, or in", span));
            };
            self.expect(&TokenKind::RParen)?;
            Ok(vis)
        } else {
            Ok(Visibility::Pub)
        }
    }
    
    /// Parse extern block
    fn parse_extern_block(&mut self, _is_unsafe: bool, _attrs: Vec<Attribute>) -> ParseResult<Item> {
        self.expect_keyword("extern")?;
        let abi = if matches!(self.current_kind(), TokenKind::StrLit(_)) {
            if let TokenKind::StrLit(s) = self.bump().value {
                s
            } else {
                String::new()
            }
        } else {
            "C".to_string()
        };
        
        self.expect(&TokenKind::LBrace)?;
        let mut items = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.at_end() {
            let item = self.parse_item()?;
            items.push(item);
            self.eat(&TokenKind::Semi);
        }
        self.expect(&TokenKind::RBrace)?;
        
        // Convert items to extern block items
        let extern_items: Vec<Item> = items.into_iter().map(|item| {
            // Mark functions as extern
            item
        }).collect();
        
        Ok(Item::ExternBlock { abi, items: extern_items })
    }
    
    /// Parse function
    fn parse_fn(&mut self, attrs: Vec<Attribute>, is_unsafe: bool, _is_default: bool) -> ParseResult<Item> {
        self.expect_keyword("fn")?;
        
        let name = self.parse_ident()?;
        
        // Parse generics
        let generics = self.parse_generics()?;
        
        // Parse parameters
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();
        let mut variadic = false;
        while !self.check(&TokenKind::RParen) && !self.at_end() {
            if self.check(&TokenKind::DotDot) {
                self.bump();
                variadic = true;
                break;
            }
            let param = self.parse_fn_param()?;
            params.push(param);
            if !self.eat(&TokenKind::Comma).is_some() {
                break;
            }
        }
        self.expect(&TokenKind::RParen)?;
        
        // Parse return type
        let return_type = if self.check(&TokenKind::Arrow) {
            self.bump();
            Some(self.parse_type()?)
        } else {
            None
        };
        
        // Parse body
        let body = if self.check(&TokenKind::LBrace) {
            Some(self.parse_block()?)
        } else {
            self.expect(&TokenKind::Semi)?;
            None
        };
        
        // Check for const/async keywords before fn (already consumed)
        let is_const = false; // Simplified
        let is_async = false; // Simplified
        
        Ok(Item::Fn(FnDecl {
            name,
            generics,
            params,
            return_type,
            body,
            attrs,
            is_const,
            is_async,
            is_unsafe,
            is_extern: false,
            abi: None,
            variadic,
        }))
    }
    
    /// Parse function parameter
    fn parse_fn_param(&mut self) -> ParseResult<FnParam> {
        let pattern = self.parse_pattern()?;
        // Self parameters (self, &self, &mut self) have an implicit type
        if !self.check(&TokenKind::Colon) {
            return Ok(FnParam { pattern, ty: Type::SelfType });
        }
        self.expect(&TokenKind::Colon)?;
        let ty = self.parse_type()?;
        Ok(FnParam { pattern, ty })
    }
    
    /// Parse a closure parameter (pattern with optional type annotation)
    fn parse_closure_param(&mut self) -> ParseResult<FnParam> {
        let pattern = self.parse_pattern()?;
        let ty = if self.check(&TokenKind::Colon) {
            self.bump();
            self.parse_type()?
        } else {
            Type::Inferred
        };
        Ok(FnParam { pattern, ty })
    }
    
    /// Parse struct
    fn parse_struct(&mut self, attrs: Vec<Attribute>) -> ParseResult<Item> {
        self.expect_keyword("struct")?;
        let name = self.parse_ident()?;
        let generics = self.parse_generics()?;
        
        let (fields, kind) = if self.check(&TokenKind::LBrace) {
            self.bump();
            let mut fields = Vec::new();
            while !self.check(&TokenKind::RBrace) && !self.at_end() {
                let field = self.parse_field()?;
                fields.push(field);
                if !self.eat(&TokenKind::Comma).is_some() {
                    break;
                }
            }
            self.expect(&TokenKind::RBrace)?;
            (fields, StructKind::Named)
        } else if self.check(&TokenKind::LParen) {
            self.bump();
            let mut fields = Vec::new();
            while !self.check(&TokenKind::RParen) && !self.at_end() {
                let ty = self.parse_type()?;
                fields.push(FieldDecl {
                    name: Ident::new(format!("_{}", fields.len())),
                    ty,
                    attrs: vec![],
                    visibility: Visibility::Private,
                });
                if !self.eat(&TokenKind::Comma).is_some() {
                    break;
                }
            }
            self.expect(&TokenKind::RParen)?;
            self.eat(&TokenKind::Semi);
            (fields, StructKind::Tuple)
        } else {
            self.expect(&TokenKind::Semi)?;
            (vec![], StructKind::Unit)
        };
        
        Ok(Item::Struct(StructDecl {
            name,
            generics,
            fields,
            kind,
            attrs,
        }))
    }
    
    /// Parse enum
    fn parse_enum(&mut self, attrs: Vec<Attribute>) -> ParseResult<Item> {
        self.expect_keyword("enum")?;
        let name = self.parse_ident()?;
        let generics = self.parse_generics()?;
        
        self.expect(&TokenKind::LBrace)?;
        let mut variants = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.at_end() {
            // Parse variant attributes
            let mut var_attrs = Vec::new();
            while self.check(&TokenKind::Pound) {
                let attr = self.parse_attribute()?;
                var_attrs.push(attr);
            }
            
            let name = self.parse_ident()?;
            let (kind, _discriminant) = if self.check(&TokenKind::LParen) {
                self.bump();
                let mut fields = Vec::new();
                while !self.check(&TokenKind::RParen) && !self.at_end() {
                    let ty = self.parse_type()?;
                    fields.push(FieldDecl {
                        name: Ident::new(format!("_{}", fields.len())),
                        ty,
                        attrs: vec![],
                        visibility: Visibility::Private,
                    });
                    if !self.eat(&TokenKind::Comma).is_some() {
                        break;
                    }
                }
                self.expect(&TokenKind::RParen)?;
                (VariantKind::Tuple(fields), None::<Expr>)
            } else if self.check(&TokenKind::LBrace) {
                self.bump();
                let mut fields = Vec::new();
                while !self.check(&TokenKind::RBrace) && !self.at_end() {
                    let field = self.parse_field()?;
                    fields.push(field);
                    if !self.eat(&TokenKind::Comma).is_some() {
                        break;
                    }
                }
                self.expect(&TokenKind::RBrace)?;
                (VariantKind::Named(fields), None)
            } else {
                (VariantKind::Unit, None)
            };
            
            let discriminant = if self.check(&TokenKind::Eq) {
                self.bump();
                Some(self.parse_expr()?)
            } else {
                None
            };
            
            variants.push(EnumVariant { name, kind, attrs: var_attrs, discriminant });
            
            if !self.eat(&TokenKind::Comma).is_some() {
                break;
            }
        }
        self.expect(&TokenKind::RBrace)?;
        
        Ok(Item::Enum(EnumDecl { name, generics, variants, attrs }))
    }
    
    /// Parse union
    fn parse_union(&mut self, attrs: Vec<Attribute>) -> ParseResult<Item> {
        self.expect_keyword("union")?;
        let name = self.parse_ident()?;
        let generics = self.parse_generics()?;
        
        self.expect(&TokenKind::LBrace)?;
        let mut fields = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.at_end() {
            let field = self.parse_field()?;
            fields.push(field);
            if !self.eat(&TokenKind::Comma).is_some() {
                break;
            }
        }
        self.expect(&TokenKind::RBrace)?;
        
        Ok(Item::Union(UnionDecl { name, generics, fields, attrs }))
    }
    
    /// Parse type alias
    fn parse_type_alias(&mut self, _attrs: Vec<Attribute>) -> ParseResult<Item> {
        self.expect_keyword("type")?;
        let name = self.parse_ident()?;
        let generics = self.parse_generics()?;
        self.expect(&TokenKind::Eq)?;
        let ty = self.parse_type()?;
        self.expect(&TokenKind::Semi)?;
        
        Ok(Item::TypeAlias { name, generics, ty })
    }
    
    /// Parse trait
    fn parse_trait(&mut self, attrs: Vec<Attribute>, is_unsafe: bool) -> ParseResult<Item> {
        self.expect_keyword("trait")?;
        let name = self.parse_ident()?;
        let generics = self.parse_generics()?;
        
        // Parse supertrait bounds
        let mut bounds = Vec::new();
        if self.check(&TokenKind::Colon) {
            self.bump();
            loop {
                let path = self.parse_path()?;
                bounds.push(path);
                if !self.eat(&TokenKind::Plus).is_some() {
                    break;
                }
            }
        }
        
        self.expect(&TokenKind::LBrace)?;
        let mut items = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.at_end() {
            let item = self.parse_trait_item()?;
            items.push(item);
            self.eat(&TokenKind::Semi);
        }
        self.expect(&TokenKind::RBrace)?;
        
        Ok(Item::Trait(TraitDecl {
            name,
            generics,
            bounds,
            items,
            attrs,
            is_unsafe,
            is_auto: false,
        }))
    }
    
    /// Parse trait item
    fn parse_trait_item(&mut self) -> ParseResult<TraitItem> {
        // Handle attributes
        let _attrs: Vec<Attribute> = if self.check(&TokenKind::Pound) {
            self.parse_attribute()?;
            Vec::new()
        } else {
            Vec::new()
        };
        
        if self.check_keyword("fn") {
            let item = self.parse_fn(vec![], false, false)?;
            if let Item::Fn(func) = item {
                return Ok(TraitItem::Fn(func));
            }
        }
        
        if self.check_keyword("type") {
            self.bump();
            let name = self.parse_ident()?;
            let mut bounds = Vec::new();
            if self.check(&TokenKind::Colon) {
                self.bump();
                loop {
                    let path = self.parse_path()?;
                    bounds.push(path);
                    if !self.eat(&TokenKind::Plus).is_some() {
                        break;
                    }
                }
            }
            let default = if self.check(&TokenKind::Eq) {
                self.bump();
                Some(self.parse_type()?)
            } else {
                None
            };
            self.expect(&TokenKind::Semi)?;
            return Ok(TraitItem::Type { name, bounds, default });
        }
        
        if self.check_keyword("const") {
            self.bump();
            let name = self.parse_ident()?;
            self.expect(&TokenKind::Colon)?;
            let ty = self.parse_type()?;
            let default = if self.check(&TokenKind::Eq) {
                self.bump();
                Some(self.parse_expr()?)
            } else {
                None
            };
            self.expect(&TokenKind::Semi)?;
            return Ok(TraitItem::Const { name, ty, default });
        }
        
        let span = self.peek().span;
        Err(ParseError::new("expected trait item", span))
    }
    
    /// Parse impl block
    fn parse_impl(&mut self, attrs: Vec<Attribute>, is_unsafe: bool) -> ParseResult<Item> {
        self.expect_keyword("impl")?;
        
        let generics = self.parse_generics()?;
        
        // Check if this is a trait impl (has "for")
        // Parse the first type
        let first_type = self.parse_type()?;
        
        let (trait_path, self_type) = if self.check_keyword("for") {
            self.bump();
            let self_type = self.parse_type()?;
            // Convert first_type to a path if possible
            let trait_path = match first_type {
                Type::Path(path, _) => path,
                _ => {
                    let span = self.peek().span;
                    return Err(ParseError::new("expected trait path in impl", span));
                }
            };
            (Some(trait_path), self_type)
        } else {
            (None, first_type)
        };
        
        self.expect(&TokenKind::LBrace)?;
        let mut items = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.at_end() {
            // Parse attributes
            let _item_attrs: Vec<Attribute> = if self.check(&TokenKind::Pound) {
                self.parse_attribute()?;
                Vec::new()
            } else {
                Vec::new()
            };
            
            let is_default = if self.check_keyword("default") {
                self.bump();
                true
            } else {
                false
            };
            
            if self.check_keyword("fn") {
                let item = self.parse_fn(vec![], is_unsafe, is_default)?;
                if let Item::Fn(func) = item {
                    items.push(ImplItem::Fn(func));
                }
                self.eat(&TokenKind::Semi);
            } else if self.check_keyword("const") {
                self.bump();
                let name = self.parse_ident()?;
                self.expect(&TokenKind::Colon)?;
                let ty = self.parse_type()?;
                self.expect(&TokenKind::Eq)?;
                let value = self.parse_expr()?;
                self.expect(&TokenKind::Semi)?;
                items.push(ImplItem::Const { name, ty, value });
            } else if self.check_keyword("type") {
                self.bump();
                let name = self.parse_ident()?;
                self.expect(&TokenKind::Eq)?;
                let ty = self.parse_type()?;
                self.expect(&TokenKind::Semi)?;
                items.push(ImplItem::Type { name, ty });
            } else {
                let span = self.peek().span;
                return Err(ParseError::new("expected impl item", span));
            }
        }
        self.expect(&TokenKind::RBrace)?;
        
        Ok(Item::Impl(ImplDecl {
            generics,
            trait_path,
            self_type,
            items,
            attrs,
            is_unsafe,
            is_default: false,
        }))
    }
    
    /// Parse module
    fn parse_mod(&mut self, _attrs: Vec<Attribute>) -> ParseResult<Item> {
        self.expect_keyword("mod")?;
        let name = self.parse_ident()?;
        
        if self.check(&TokenKind::Semi) {
            self.bump();
            return Ok(Item::Mod { name, items: vec![] });
        }
        
        self.expect(&TokenKind::LBrace)?;
        let mut items = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.at_end() {
            let item = self.parse_item()?;
            items.push(item);
            self.eat(&TokenKind::Semi);
        }
        self.expect(&TokenKind::RBrace)?;
        
        Ok(Item::Mod { name, items })
    }
    
    /// Parse use/import
    fn parse_use(&mut self, _attrs: Vec<Attribute>) -> ParseResult<Item> {
        self.expect_keyword("use")?;
        
        let path = self.parse_path()?;
        
        let mut is_glob = false;
        let mut items = Vec::new();
        let mut alias = None;
        
        if self.check(&TokenKind::DoubleColon) {
            self.bump();
            if self.check(&TokenKind::Star) {
                self.bump();
                is_glob = true;
            } else if self.check(&TokenKind::LBrace) {
                self.bump();
                while !self.check(&TokenKind::RBrace) && !self.at_end() {
                    let item_path = self.parse_path()?;
                    let item_alias = if self.check_keyword("as") {
                        self.bump();
                        Some(self.parse_ident()?)
                    } else {
                        None
                    };
                    items.push(UseTree { path: item_path, alias: item_alias, is_glob: false });
                    if !self.eat(&TokenKind::Comma).is_some() {
                        break;
                    }
                }
                self.expect(&TokenKind::RBrace)?;
            }
        } else if self.check_keyword("as") {
            self.bump();
            alias = Some(self.parse_ident()?);
        }
        
        self.expect(&TokenKind::Semi)?;
        
        Ok(Item::Use(UseDecl { path, alias, visibility: Visibility::Private, is_glob, items }))
    }
    
    /// Parse const or static
    fn parse_const(&mut self, _attrs: Vec<Attribute>, is_static: bool) -> ParseResult<Item> {
        if is_static {
            self.expect_keyword("static")?;
        } else {
            self.expect_keyword("const")?;
        }
        
        let is_mutable = if is_static && self.check_keyword("mut") {
            self.bump();
            true
        } else {
            false
        };
        
        let name = self.parse_ident()?;
        self.expect(&TokenKind::Colon)?;
        let ty = self.parse_type()?;
        self.expect(&TokenKind::Eq)?;
        let value = self.parse_expr()?;
        self.expect(&TokenKind::Semi)?;
        
        if is_static {
            Ok(Item::Static { name, ty, value, mutable: is_mutable })
        } else {
            Ok(Item::Const { name, ty, value })
        }
    }
    
    /// Parse macro definition
    fn parse_macro_def(&mut self, _attrs: Vec<Attribute>) -> ParseResult<Item> {
        self.expect_keyword("macro")?;
        let name = self.parse_ident()?;
        self.expect(&TokenKind::Pound)?; // !
        
        let body = self.parse_macro_body()?;
        
        Ok(Item::MacroDef { name, body })
    }
    
    /// Parse macro_rules!
    fn parse_macro_rules(&mut self, _attrs: Vec<Attribute>) -> ParseResult<Item> {
        // macro_rules! name { rules }
        let _ = self.parse_ident()?; // macro_rules
        self.expect(&TokenKind::Not)?;
        let name = self.parse_ident()?;
        
        let body = self.parse_macro_body()?;
        
        Ok(Item::MacroDef { name, body })
    }
    
    /// Parse macro body
    fn parse_macro_body(&mut self) -> ParseResult<MacroBody> {
        if self.check(&TokenKind::LBrace) {
            self.bump();
            let mut tokens = String::new();
            let mut depth = 1;
            while depth > 0 && !self.at_end() {
                match self.current_kind() {
                    TokenKind::LBrace => { depth += 1; tokens.push_str("{"); }
                    TokenKind::RBrace => { depth -= 1; if depth > 0 { tokens.push_str("}"); } }
                    _ => tokens.push_str(&self.current_kind().to_string()),
                }
                self.bump();
            }
            Ok(MacroBody::Tokens(tokens))
        } else {
            let span = self.peek().span;
            Err(ParseError::new("expected macro body", span))
        }
    }
    
    /// Parse field declaration
    fn parse_field(&mut self) -> ParseResult<FieldDecl> {
        let visibility = if self.check_keyword("pub") {
            self.parse_visibility()?
        } else {
            Visibility::Private
        };
        
        let name = self.parse_ident()?;
        self.expect(&TokenKind::Colon)?;
        let ty = self.parse_type()?;
        
        Ok(FieldDecl { name, ty, attrs: vec![], visibility })
    }
    
    /// Parse identifier
    fn parse_ident(&mut self) -> ParseResult<Ident> {
        match self.bump().value {
            TokenKind::Ident(s) => Ok(Ident::new(s)),
            TokenKind::KwSelf => Ok(Ident::new("self")),
            TokenKind::KwSelfType => Ok(Ident::new("Self")),
            TokenKind::KwCrate => Ok(Ident::new("crate")),
            TokenKind::KwSuper => Ok(Ident::new("super")),
            other => {
                let span = self.peek().span;
                Err(ParseError::new(format!("expected identifier, found {}", other), span))
            }
        }
    }
    
    /// Parse path (a::b::c)
    fn parse_path(&mut self) -> ParseResult<Path> {
        let mut segments = Vec::new();
        
        // Check for crate-relative
        let is_crate_relative = if self.check_keyword("crate") {
            self.bump();
            true
        } else {
            false
        };
        
        if is_crate_relative {
            // Already consumed crate, now expect ::
            if self.check(&TokenKind::DoubleColon) {
                self.bump();
            }
        }
        
        // First segment
        let first = match self.bump().value {
            TokenKind::Ident(s) => Ident::new(s),
            TokenKind::KwSelf => Ident::new("self"),
            TokenKind::KwSelfType => Ident::new("Self"),
            TokenKind::KwSuper => Ident::new("super"),
            TokenKind::KwCrate => Ident::new("crate"),
            // Primitive type keywords can also be path segments (e.g. i32::MAX)
            other => {
                if let Some(kw) = other.keyword_str() {
                    Ident::new(kw)
                } else {
                    let span = self.peek().span;
                    return Err(ParseError::new(format!("expected path, found {}", other), span));
                }
            }
        };
        segments.push(first);
        
        // Subsequent segments
        while self.check(&TokenKind::DoubleColon) {
            // Peek: only continue if the token after :: can start a segment
            // (not { or * which are used by use declarations)
            if !Self::can_start_path_segment(&self.peek_at(1).value) {
                break;
            }
            self.bump();
            let seg = match self.bump().value {
                TokenKind::Ident(s) => Ident::new(s),
                TokenKind::KwSelf => Ident::new("self"),
                TokenKind::KwSelfType => Ident::new("Self"),
                TokenKind::KwSuper => Ident::new("super"),
                TokenKind::KwCrate => Ident::new("crate"),
                other => {
                    if let Some(kw) = other.keyword_str() {
                        Ident::new(kw)
                    } else {
                        let span = self.peek().span;
                        return Err(ParseError::new(format!("expected path segment, found {}", other), span));
                    }
                }
            };
            segments.push(seg);
        }
        
        Ok(Path { segments, is_crate_relative })
    }
    
    /// Check if a token can start a path segment
    fn can_start_path_segment(kind: &TokenKind) -> bool {
        matches!(kind, TokenKind::Ident(_))
            || matches!(kind, TokenKind::KwSelf | TokenKind::KwSelfType |
                TokenKind::KwSuper | TokenKind::KwCrate)
            || kind.is_primitive_type()
    }
    
    /// Parse generic parameters
    fn parse_generics(&mut self) -> ParseResult<Vec<GenericParam>> {
        if !self.check(&TokenKind::Lt) {
            return Ok(vec![]);
        }
        
        self.bump();
        let mut params = Vec::new();
        
        while !self.check(&TokenKind::Gt) && !self.at_end() {
            let param = self.parse_generic_param()?;
            params.push(param);
            if !self.eat(&TokenKind::Comma).is_some() {
                break;
            }
        }
        
        self.expect(&TokenKind::Gt)?;
        Ok(params)
    }
    
    /// Parse a single generic parameter
    fn parse_generic_param(&mut self) -> ParseResult<GenericParam> {
        // Lifetime parameter
        if self.check(&TokenKind::Not) {
            // 'a lifetime
            let lifetime = self.parse_lifetime()?;
            let mut bounds = Vec::new();
            if self.check(&TokenKind::Colon) {
                self.bump();
                loop {
                    let path = self.parse_path()?;
                    bounds.push(path);
                    if !self.eat(&TokenKind::Plus).is_some() {
                        break;
                    }
                }
            }
            return Ok(GenericParam::Lifetime { name: lifetime, bounds });
        }
        
        // Const parameter
        if self.check_keyword("const") {
            self.bump();
            let name = self.parse_ident()?;
            self.expect(&TokenKind::Colon)?;
            let ty = self.parse_type()?;
            return Ok(GenericParam::Const { name, ty });
        }
        
        // Type parameter
        let name = self.parse_ident()?;
        let mut bounds = Vec::new();
        let mut default = None;
        
        if self.check(&TokenKind::Colon) {
            self.bump();
            loop {
                let path = self.parse_path()?;
                bounds.push(path);
                if !self.eat(&TokenKind::Plus).is_some() {
                    break;
                }
            }
        }
        
        if self.check(&TokenKind::Eq) {
            self.bump();
            default = Some(self.parse_type()?);
        }
        
        Ok(GenericParam::Type { name, bounds, default })
    }
    
    /// Parse lifetime
    fn parse_lifetime(&mut self) -> ParseResult<Lifetime> {
        // Simplified: lifetime is a ' followed by identifier
        // In our token scheme, we treat it specially
        if self.check(&TokenKind::Not) {
            self.bump();
            let name = self.parse_ident()?;
            return Ok(Lifetime(format!("'{}", name.0)));
        }
        let span = self.peek().span;
        Err(ParseError::new("expected lifetime", span))
    }
    
    /// Parse a block { ... }
    pub fn parse_block(&mut self) -> ParseResult<Block> {
        self.expect(&TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        
        while !self.check(&TokenKind::RBrace) && !self.at_end() {
            let stmt = self.parse_stmt()?;
            stmts.push(stmt);
        }
        
        self.expect(&TokenKind::RBrace)?;
        Ok(Block { stmts })
    }
    
    /// Parse a statement
    pub fn parse_stmt(&mut self) -> ParseResult<Stmt> {
        // Handle labels
        if self.check(&TokenKind::Not) {
            // Label: 'label:
            // Simplified handling
        }
        
        // let statement
        if self.check_keyword("let") {
            self.bump();
            let mutable = if self.check_keyword("mut") {
                self.bump();
                true
            } else {
                false
            };
            let pattern = self.parse_pattern()?;
            let ty = if self.check(&TokenKind::Colon) {
                self.bump();
                Some(self.parse_type()?)
            } else {
                None
            };
            let value = if self.check(&TokenKind::Eq) {
                self.bump();
                Some(self.parse_expr()?)
            } else {
                None
            };
            self.expect(&TokenKind::Semi)?;
            return Ok(Stmt::Let { pattern, ty, value, mutable });
        }
        
        // Item declarations
        if self.check_keyword("fn") || self.check_keyword("struct") ||
           self.check_keyword("enum") || self.check_keyword("union") ||
           self.check_keyword("type") || self.check_keyword("trait") ||
           self.check_keyword("impl") || self.check_keyword("mod") ||
           self.check_keyword("use") || self.check_keyword("const") ||
           self.check_keyword("static") || self.check_keyword("extern") ||
           self.check_keyword("unsafe") || self.check_keyword("pub") {
            let item = self.parse_item()?;
            return Ok(Stmt::Item(item));
        }
        
        // Expression statement
        let expr = self.parse_expr()?;
        // Semi is optional for block-like expressions and for tail expressions
        // (the final expression of a block, followed by `}`)
        let needs_semi = !matches!(expr, Expr::If { .. } | Expr::Match { .. } | Expr::Loop(..) |
            Expr::While { .. } | Expr::For { .. } | Expr::Block(..) |
            Expr::Unsafe(..) | Expr::Async(..))
            && !self.check(&TokenKind::RBrace)
            && !self.at_end();
        if needs_semi {
            self.expect(&TokenKind::Semi)?;
        }
        Ok(Stmt::Expr(expr))
    }
    
    /// Parse a pattern
    pub fn parse_pattern(&mut self) -> ParseResult<Pattern> {
        self.parse_or_pattern()
    }
    
    /// Parse or-pattern (a | b)
    fn parse_or_pattern(&mut self) -> ParseResult<Pattern> {
        let mut patterns = vec![self.parse_pattern_atom()?];
        while self.check(&TokenKind::Or) {
            self.bump();
            patterns.push(self.parse_pattern_atom()?);
        }
        if patterns.len() == 1 {
            Ok(patterns.pop().unwrap())
        } else {
            Ok(Pattern::Or(patterns))
        }
    }
    
    /// Parse a pattern atom
    fn parse_pattern_atom(&mut self) -> ParseResult<Pattern> {
        // Wildcard
        if self.check(&TokenKind::Underscore) {
            self.bump();
            return Ok(Pattern::Wildcard);
        }
        
        // Reference pattern
        if self.check(&TokenKind::And) {
            self.bump();
            let mutable = if self.check_keyword("mut") {
                self.bump();
                true
            } else {
                false
            };
            let pattern = self.parse_pattern()?;
            return Ok(Pattern::Reference { mutable, pattern: Box::new(pattern) });
        }
        
        // Box pattern
        if self.check_keyword("box") {
            self.bump();
            let pattern = self.parse_pattern()?;
            return Ok(Pattern::Box(Box::new(pattern)));
        }
        
        // Literal
        match self.current_kind().clone() {
            TokenKind::IntLit(n, s) => {
                self.bump();
                return Ok(Pattern::Literal(Literal::Int(n, s)));
            }
            TokenKind::FloatLit(n, s) => {
                self.bump();
                return Ok(Pattern::Literal(Literal::Float(n, s)));
            }
            TokenKind::CharLit(c) => {
                self.bump();
                return Ok(Pattern::Literal(Literal::Char(c)));
            }
            TokenKind::StrLit(s) => {
                self.bump();
                return Ok(Pattern::Literal(Literal::Str(s)));
            }
            TokenKind::BoolLit(b) => {
                self.bump();
                return Ok(Pattern::Literal(Literal::Bool(b)));
            }
            _ => {}
        }
        
        // Range pattern: start..end or start..=end
        // (handled after parsing atom)
        
        // Parenthesized or tuple pattern
        if self.check(&TokenKind::LParen) {
            self.bump();
            let mut patterns = Vec::new();
            while !self.check(&TokenKind::RParen) && !self.at_end() {
                let p = self.parse_pattern()?;
                patterns.push(p);
                if !self.eat(&TokenKind::Comma).is_some() {
                    break;
                }
            }
            self.expect(&TokenKind::RParen)?;
            if patterns.len() == 1 {
                return Ok(patterns.pop().unwrap());
            }
            return Ok(Pattern::Tuple(patterns));
        }
        
        // Slice pattern
        if self.check(&TokenKind::LBracket) {
            self.bump();
            let mut patterns = Vec::new();
            let mut rest = None;
            while !self.check(&TokenKind::RBracket) && !self.at_end() {
                let p = self.parse_pattern()?;
                if self.check(&TokenKind::DotDot) {
                    self.bump();
                    rest = Some(Box::new(p));
                    break;
                }
                patterns.push(p);
                if !self.eat(&TokenKind::Comma).is_some() {
                    break;
                }
            }
            self.expect(&TokenKind::RBracket)?;
            return Ok(Pattern::Slice { patterns, rest });
        }
        
        // Struct pattern or path pattern or tuple struct pattern
        if matches!(self.current_kind(), TokenKind::Ident(_)) || self.check_keyword("self") ||
           self.check_keyword("Self") || self.check_keyword("crate") ||
           self.check_keyword("super") {
            let path = self.parse_path()?;
            
            // Tuple struct pattern
            if self.check(&TokenKind::LParen) {
                self.bump();
                let mut patterns = Vec::new();
                while !self.check(&TokenKind::RParen) && !self.at_end() {
                    let p = self.parse_pattern()?;
                    patterns.push(p);
                    if !self.eat(&TokenKind::Comma).is_some() {
                        break;
                    }
                }
                self.expect(&TokenKind::RParen)?;
                return Ok(Pattern::TupleStruct { path, patterns });
            }
            
            // Struct pattern
            if self.check(&TokenKind::LBrace) {
                self.bump();
                let mut fields = Vec::new();
                while !self.check(&TokenKind::RBrace) && !self.at_end() {
                    let name = self.parse_ident()?;
                    let field_pattern = if self.check(&TokenKind::Colon) {
                        self.bump();
                        self.parse_pattern()?
                    } else {
                        // Shorthand
                        Pattern::Ident {
                            name: name.clone(),
                            mutable: false,
                            subpattern: None,
                        }
                    };
                    fields.push(FieldPattern { name, pattern: field_pattern, shorthand: false });
                    if !self.eat(&TokenKind::Comma).is_some() {
                        break;
                    }
                }
                self.expect(&TokenKind::RBrace)?;
                return Ok(Pattern::Struct { path, fields });
            }
            
            // @ binding
            if self.check(&TokenKind::At) {
                self.bump();
                let sub = self.parse_pattern()?;
                // The path becomes the name
                if path.segments.len() == 1 {
                    return Ok(Pattern::At {
                        name: path.segments[0].clone(),
                        pattern: Box::new(sub),
                    });
                }
            }
            
            // Identifier binding (with optional mut and @ subpattern)
            if path.segments.len() == 1 {
                let name = path.segments[0].clone();
                // Check for mut (already handled in let, but for safety)
                // Check for @ subpattern
                let subpattern = if self.check(&TokenKind::At) {
                    self.bump();
                    Some(Box::new(self.parse_pattern()?))
                } else {
                    None
                };
                return Ok(Pattern::Ident { name, mutable: false, subpattern });
            }
            
            return Ok(Pattern::Path(path));
        }
        
        let span = self.peek().span;
        Err(ParseError::new(format!("expected pattern, found {}", self.current_kind()), span))
    }
    
    /// Parse a type
    pub fn parse_type(&mut self) -> ParseResult<Type> {
        // Handle parenthesized types (tuple)
        if self.check(&TokenKind::LParen) {
            self.bump();
            let mut types = Vec::new();
            while !self.check(&TokenKind::RParen) && !self.at_end() {
                let ty = self.parse_type()?;
                types.push(ty);
                if !self.eat(&TokenKind::Comma).is_some() {
                    break;
                }
            }
            self.expect(&TokenKind::RParen)?;
            if types.is_empty() {
                return Ok(Type::Unit);
            }
            if types.len() == 1 {
                return Ok(types.pop().unwrap());
            }
            return Ok(Type::Tuple(types));
        }
        
        // Handle reference types
        if self.check(&TokenKind::And) {
            self.bump();
            // Lifetime (optional)
            let lifetime = None; // Simplified
            let mutable = if self.check_keyword("mut") {
                self.bump();
                true
            } else {
                false
            };
            let pointee = self.parse_type()?;
            return Ok(Type::Reference {
                mutable,
                lifetime,
                pointee: Box::new(pointee),
            });
        }
        
        // Handle raw pointer types
        if self.check(&TokenKind::Star) {
            self.bump();
            let mutable = if self.check_keyword("mut") {
                self.bump();
                true
            } else {
                self.expect_keyword("const")?;
                false
            };
            let pointee = self.parse_type()?;
            return Ok(Type::RawPointer {
                mutable,
                pointee: Box::new(pointee),
            });
        }
        
        // Handle array types
        if self.check(&TokenKind::LBracket) {
            self.bump();
            let element = self.parse_type()?;
            let length = if self.check(&TokenKind::Semi) {
                self.bump();
                let expr = self.parse_expr()?;
                self.expect(&TokenKind::RBracket)?;
                Some(Box::new(expr))
            } else {
                self.expect(&TokenKind::RBracket)?;
                None
            };
            return Ok(Type::Array {
                element: Box::new(element),
                length,
            });
        }
        
        // Handle never type
        if self.check(&TokenKind::Not) {
            self.bump();
            return Ok(Type::Never);
        }
        
        // Handle underscore (inferred)
        if self.check(&TokenKind::Underscore) {
            self.bump();
            return Ok(Type::Inferred);
        }
        
        // Handle impl trait
        if self.check_keyword("impl") {
            self.bump();
            let mut paths = Vec::new();
            loop {
                let path = self.parse_path()?;
                paths.push(path);
                if !self.eat(&TokenKind::Plus).is_some() {
                    break;
                }
            }
            return Ok(Type::ImplTrait(paths));
        }
        
        // Handle dyn trait
        if self.check_keyword("dyn") {
            self.bump();
            let mut paths = Vec::new();
            loop {
                let path = self.parse_path()?;
                paths.push(path);
                if !self.eat(&TokenKind::Plus).is_some() {
                    break;
                }
            }
            return Ok(Type::TraitObject(paths));
        }
        
        // Handle Self
        if self.check_keyword("Self") {
            self.bump();
            return Ok(Type::SelfType);
        }
        
        // Handle primitive or path type
        let path = self.parse_path()?;
        
        // Check for generic arguments
        if self.check(&TokenKind::Lt) {
            self.bump();
            let mut args = Vec::new();
            while !self.check(&TokenKind::Gt) && !self.at_end() {
                let arg = self.parse_type()?;
                args.push(arg);
                if !self.eat(&TokenKind::Comma).is_some() {
                    break;
                }
            }
            self.expect(&TokenKind::Gt)?;
            return Ok(Type::Path(path, args));
        }
        
        // Check for nullable type
        if self.check(&TokenKind::Question) {
            self.bump();
            return Ok(Type::Nullable(Box::new(Type::Path(path, vec![]))));
        }
        
        // Single-segment primitive type
        if path.segments.len() == 1 {
            let name = &path.segments[0].0;
            let primitives = ["bool", "char", "str", "i8", "i16", "i32", "i64", "i128",
                "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64"];
            if primitives.contains(&name.as_str()) {
                return Ok(Type::Primitive(path.segments[0].clone()));
            }
        }
        
        Ok(Type::Path(path, vec![]))
    }
    
    /// Parse an expression
    pub fn parse_expr(&mut self) -> ParseResult<Expr> {
        self.parse_assign_expr()
    }
    
    /// Parse assignment expression (lowest precedence)
    fn parse_assign_expr(&mut self) -> ParseResult<Expr> {
        let left = self.parse_range_expr()?;
        
        match self.current_kind() {
            TokenKind::Eq => {
                self.bump();
                let right = self.parse_assign_expr()?;
                Ok(Expr::Assign { lhs: Box::new(left), rhs: Box::new(right) })
            }
            TokenKind::PlusEq => {
                self.bump();
                let right = self.parse_assign_expr()?;
                Ok(Expr::AssignOp { op: BinOp::Add, lhs: Box::new(left), rhs: Box::new(right) })
            }
            TokenKind::MinusEq => {
                self.bump();
                let right = self.parse_assign_expr()?;
                Ok(Expr::AssignOp { op: BinOp::Sub, lhs: Box::new(left), rhs: Box::new(right) })
            }
            TokenKind::StarEq => {
                self.bump();
                let right = self.parse_assign_expr()?;
                Ok(Expr::AssignOp { op: BinOp::Mul, lhs: Box::new(left), rhs: Box::new(right) })
            }
            TokenKind::SlashEq => {
                self.bump();
                let right = self.parse_assign_expr()?;
                Ok(Expr::AssignOp { op: BinOp::Div, lhs: Box::new(left), rhs: Box::new(right) })
            }
            TokenKind::PercentEq => {
                self.bump();
                let right = self.parse_assign_expr()?;
                Ok(Expr::AssignOp { op: BinOp::Rem, lhs: Box::new(left), rhs: Box::new(right) })
            }
            TokenKind::AndEq => {
                self.bump();
                let right = self.parse_assign_expr()?;
                Ok(Expr::AssignOp { op: BinOp::BitAnd, lhs: Box::new(left), rhs: Box::new(right) })
            }
            TokenKind::OrEq => {
                self.bump();
                let right = self.parse_assign_expr()?;
                Ok(Expr::AssignOp { op: BinOp::BitOr, lhs: Box::new(left), rhs: Box::new(right) })
            }
            TokenKind::XorEq => {
                self.bump();
                let right = self.parse_assign_expr()?;
                Ok(Expr::AssignOp { op: BinOp::BitXor, lhs: Box::new(left), rhs: Box::new(right) })
            }
            _ => Ok(left),
        }
    }
    
    /// Parse range expression
    fn parse_range_expr(&mut self) -> ParseResult<Expr> {
        let start = self.parse_or_expr()?;
        
        if self.check(&TokenKind::DotDot) {
            self.bump();
            let inclusive = if self.check(&TokenKind::Eq) {
                self.bump();
                true
            } else {
                false
            };
            let end = if self.check_any(&[
                TokenKind::RBrace, TokenKind::RBracket, TokenKind::RParen,
                TokenKind::Comma, TokenKind::Semi, TokenKind::Eof,
            ]) {
                None
            } else {
                Some(Box::new(self.parse_assign_expr()?))
            };
            Ok(Expr::Range { start: Some(Box::new(start)), end, inclusive })
        } else if self.check(&TokenKind::DotDotEq) {
            self.bump();
            let end = Some(Box::new(self.parse_assign_expr()?));
            Ok(Expr::Range { start: Some(Box::new(start)), end, inclusive: true })
        } else {
            Ok(start)
        }
    }
    
    /// Parse || expression
    fn parse_or_expr(&mut self) -> ParseResult<Expr> {
        let mut left = self.parse_and_expr()?;
        while self.check(&TokenKind::OrOr) {
            self.bump();
            let right = self.parse_and_expr()?;
            left = Expr::Binary { op: BinOp::Or, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }
    
    /// Parse && expression
    fn parse_and_expr(&mut self) -> ParseResult<Expr> {
        let mut left = self.parse_cmp_expr()?;
        while self.check(&TokenKind::AndAnd) {
            self.bump();
            let right = self.parse_cmp_expr()?;
            left = Expr::Binary { op: BinOp::And, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }
    
    /// Parse comparison expression
    fn parse_cmp_expr(&mut self) -> ParseResult<Expr> {
        let mut left = self.parse_bit_or_expr()?;
        loop {
            let op = match self.current_kind() {
                TokenKind::EqEq => BinOp::Eq,
                TokenKind::NotEq => BinOp::Ne,
                TokenKind::Lt => BinOp::Lt,
                TokenKind::Gt => BinOp::Gt,
                TokenKind::LtEq => BinOp::Le,
                TokenKind::GtEq => BinOp::Ge,
                _ => break,
            };
            self.bump();
            let right = self.parse_bit_or_expr()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }
    
    /// Parse | expression
    fn parse_bit_or_expr(&mut self) -> ParseResult<Expr> {
        let mut left = self.parse_bit_xor_expr()?;
        while self.check(&TokenKind::Or) {
            self.bump();
            let right = self.parse_bit_xor_expr()?;
            left = Expr::Binary { op: BinOp::BitOr, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }
    
    /// Parse ^ expression
    fn parse_bit_xor_expr(&mut self) -> ParseResult<Expr> {
        let mut left = self.parse_bit_and_expr()?;
        while self.check(&TokenKind::Xor) {
            self.bump();
            let right = self.parse_bit_and_expr()?;
            left = Expr::Binary { op: BinOp::BitXor, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }
    
    /// Parse & expression
    fn parse_bit_and_expr(&mut self) -> ParseResult<Expr> {
        let mut left = self.parse_shift_expr()?;
        while self.check(&TokenKind::And) {
            self.bump();
            let right = self.parse_shift_expr()?;
            left = Expr::Binary { op: BinOp::BitAnd, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }
    
    /// Parse << >> expression
    fn parse_shift_expr(&mut self) -> ParseResult<Expr> {
        let mut left = self.parse_add_expr()?;
        loop {
            let op = match self.current_kind() {
                TokenKind::Shl => BinOp::Shl,
                TokenKind::Shr => BinOp::Shr,
                _ => break,
            };
            self.bump();
            let right = self.parse_add_expr()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }
    
    /// Parse + - expression
    fn parse_add_expr(&mut self) -> ParseResult<Expr> {
        let mut left = self.parse_mul_expr()?;
        loop {
            let op = match self.current_kind() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let right = self.parse_mul_expr()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }
    
    /// Parse * / % expression
    fn parse_mul_expr(&mut self) -> ParseResult<Expr> {
        let mut left = self.parse_unary_expr()?;
        loop {
            let op = match self.current_kind() {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Percent => BinOp::Rem,
                _ => break,
            };
            self.bump();
            let right = self.parse_unary_expr()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }
    
    /// Parse unary expression
    fn parse_unary_expr(&mut self) -> ParseResult<Expr> {
        match self.current_kind() {
            TokenKind::Minus => {
                self.bump();
                let operand = self.parse_unary_expr()?;
                Ok(Expr::Unary { op: UnOp::Neg, operand: Box::new(operand) })
            }
            TokenKind::Not => {
                self.bump();
                let operand = self.parse_unary_expr()?;
                Ok(Expr::Unary { op: UnOp::Not, operand: Box::new(operand) })
            }
            TokenKind::NotBit => {
                self.bump();
                let operand = self.parse_unary_expr()?;
                Ok(Expr::Unary { op: UnOp::BitNot, operand: Box::new(operand) })
            }
            TokenKind::Star => {
                self.bump();
                let operand = self.parse_unary_expr()?;
                Ok(Expr::Deref(Box::new(operand)))
            }
            TokenKind::And => {
                self.bump();
                let mutable = if self.check_keyword("mut") {
                    self.bump();
                    true
                } else {
                    false
                };
                let operand = self.parse_unary_expr()?;
                Ok(Expr::Reference { mutable, expr: Box::new(operand) })
            }
            TokenKind::Plus => {
                self.bump();
                self.parse_unary_expr()
            }
            _ => self.parse_postfix_expr(),
        }
    }
    
    /// Parse postfix expression (calls, field access, index, etc.)
    fn parse_postfix_expr(&mut self) -> ParseResult<Expr> {
        let mut expr = self.parse_prefix_expr()?;
        
        loop {
            match self.current_kind() {
                TokenKind::LParen => {
                    self.bump();
                    let mut args = Vec::new();
                    while !self.check(&TokenKind::RParen) && !self.at_end() {
                        let arg = self.parse_expr()?;
                        args.push(arg);
                        if !self.eat(&TokenKind::Comma).is_some() {
                            break;
                        }
                    }
                    self.expect(&TokenKind::RParen)?;
                    expr = Expr::Call { callee: Box::new(expr), args };
                }
                TokenKind::LBracket => {
                    self.bump();
                    let index = self.parse_expr()?;
                    self.expect(&TokenKind::RBracket)?;
                    expr = Expr::Index { array: Box::new(expr), index: Box::new(index) };
                }
                TokenKind::Dot => {
                    self.bump();
                    // Check for tuple field access (x.0, x.1)
                    if let TokenKind::IntLit(n, _) = self.current_kind() {
                        let n = *n as usize;
                        self.bump();
                        expr = Expr::TupleField { object: Box::new(expr), index: n };
                        continue;
                    }
                    // Check for await
                    if self.check_keyword("await") {
                        self.bump();
                        expr = Expr::Await(Box::new(expr));
                        continue;
                    }
                    let field = self.parse_ident()?;
                    // Method call if followed by (
                    if self.check(&TokenKind::LParen) {
                        self.bump();
                        let mut args = Vec::new();
                        while !self.check(&TokenKind::RParen) && !self.at_end() {
                            let arg = self.parse_expr()?;
                            args.push(arg);
                            if !self.eat(&TokenKind::Comma).is_some() {
                                break;
                            }
                        }
                        self.expect(&TokenKind::RParen)?;
                        expr = Expr::MethodCall { receiver: Box::new(expr), method: field, args };
                    } else {
                        expr = Expr::Field { object: Box::new(expr), field };
                    }
                }
                TokenKind::Question => {
                    self.bump();
                    expr = Expr::Try(Box::new(expr));
                }
                TokenKind::KwAs => {
                    self.bump();
                    let target = self.parse_type()?;
                    expr = Expr::Cast { expr: Box::new(expr), target };
                }
                _ => break,
            }
        }
        
        Ok(expr)
    }
    
    /// Parse a macro invocation: name!(...), name![...], name!{...}
    fn parse_macro_invocation(&mut self) -> ParseResult<Expr> {
        let name = self.parse_ident()?;
        self.expect(&TokenKind::Not)?; // !
        
        // Determine delimiter
        let close = match self.current_kind() {
            TokenKind::LParen => TokenKind::RParen,
            TokenKind::LBracket => TokenKind::RBracket,
            TokenKind::LBrace => TokenKind::RBrace,
            _ => {
                let span = self.peek().span;
                return Err(ParseError::new(
                    "expected '(', '[' or '{' after macro name",
                    span,
                ));
            }
        };
        self.bump(); // consume opening delimiter
        
        // Helper closure-ish: parse a comma-separated expression list up to `close`
        // (inlined below per macro)
        
        match name.0.as_str() {
            "println" | "print" => {
                let newline = name.0 == "println";
                let template = if let TokenKind::StrLit(s) = self.current_kind().clone() {
                    self.bump();
                    s
                } else {
                    String::new()
                };
                let mut args = Vec::new();
                if self.check(&TokenKind::Comma) {
                    self.bump();
                }
                while !self.check(&close) && !self.at_end() {
                    args.push(self.parse_expr()?);
                    if self.eat(&TokenKind::Comma).is_none() {
                        break;
                    }
                }
                self.expect(&close)?;
                return Ok(Expr::Print { newline, template, args });
            }
            "format" => {
                let template = if let TokenKind::StrLit(s) = self.current_kind().clone() {
                    self.bump();
                    s
                } else {
                    String::new()
                };
                let mut args = Vec::new();
                if self.check(&TokenKind::Comma) {
                    self.bump();
                }
                while !self.check(&close) && !self.at_end() {
                    args.push(self.parse_expr()?);
                    if self.eat(&TokenKind::Comma).is_none() {
                        break;
                    }
                }
                self.expect(&close)?;
                return Ok(Expr::Format { template, args });
            }
            "panic" => {
                let arg = if self.check(&close) {
                    None
                } else {
                    Some(Box::new(self.parse_expr()?))
                };
                self.expect(&close)?;
                return Ok(Expr::Panic(arg));
            }
            "vec" => {
                let mut items = Vec::new();
                while !self.check(&close) && !self.at_end() {
                    items.push(self.parse_expr()?);
                    if self.eat(&TokenKind::Comma).is_none() {
                        break;
                    }
                }
                self.expect(&close)?;
                return Ok(Expr::VecLiteral(items));
            }
            _ => {
                // Generic macro: comma-separated expression list
                let mut args = Vec::new();
                while !self.check(&close) && !self.at_end() {
                    args.push(self.parse_expr()?);
                    if self.eat(&TokenKind::Comma).is_none() {
                        break;
                    }
                }
                self.expect(&close)?;
                return Ok(Expr::MacroCall { name, args });
            }
        }
    }
    
    /// Parse prefix expression (literals, if, match, etc.)
    fn parse_prefix_expr(&mut self) -> ParseResult<Expr> {
        // Macro invocation: name!(...), name![...], name!{...}
        if let TokenKind::Ident(_) = self.current_kind() {
            if self.peek_at(1).value == TokenKind::Not {
                return self.parse_macro_invocation();
            }
        }
        
        // Literals
        match self.current_kind().clone() {
            TokenKind::IntLit(n, s) => {
                self.bump();
                return Ok(Expr::Literal(Literal::Int(n, s)));
            }
            TokenKind::FloatLit(n, s) => {
                self.bump();
                return Ok(Expr::Literal(Literal::Float(n, s)));
            }
            TokenKind::CharLit(c) => {
                self.bump();
                return Ok(Expr::Literal(Literal::Char(c)));
            }
            TokenKind::StrLit(s) => {
                self.bump();
                return Ok(Expr::Literal(Literal::Str(s)));
            }
            TokenKind::BoolLit(b) => {
                self.bump();
                return Ok(Expr::Literal(Literal::Bool(b)));
            }
            _ => {}
        }
        
        // Identifiers and paths
        if matches!(self.current_kind(), TokenKind::Ident(_)) || self.check_keyword("self") ||
           self.check_keyword("Self") || self.check_keyword("crate") ||
           self.check_keyword("super") {
            let path = self.parse_path()?;
            // Struct literal: Path { fields } or Self { fields }
            if self.check(&TokenKind::LBrace) && self.no_struct_literal == 0
                && path.segments.len() == 1 && !path.is_crate_relative {
                let name = &path.segments[0].0;
                // self cannot be a struct literal type
                if name != "self" {
                    self.bump(); // consume {
                    let mut fields = Vec::new();
                    while !self.check(&TokenKind::RBrace) && !self.at_end() {
                        let fname = self.parse_ident()?;
                        let value = if self.check(&TokenKind::Colon) {
                            self.bump();
                            self.parse_expr()?
                        } else {
                            // Shorthand: field name is also the value
                            Expr::Ident(fname.clone())
                        };
                        let shorthand = matches!(&value, Expr::Ident(id) if id.0 == fname.0);
                        fields.push(FieldValue { name: fname, value, shorthand });
                        if !self.eat(&TokenKind::Comma).is_some() {
                            break;
                        }
                    }
                    self.expect(&TokenKind::RBrace)?;
                    return Ok(Expr::StructLit { path, fields });
                }
            }
            if path.segments.len() == 1 && !path.is_crate_relative {
                // Could be self/Self/crate/super
                let name = &path.segments[0].0;
                match name.as_str() {
                    "self" => return Ok(Expr::SelfExpr),
                    "Self" => return Ok(Expr::SelfType),
                    _ => {}
                }
                return Ok(Expr::Ident(path.segments[0].clone()));
            }
            return Ok(Expr::Path(path));
        }
        
        // if expression
        if self.check_keyword("if") {
            self.bump();
            self.no_struct_literal += 1;
            let cond = self.parse_expr()?;
            self.no_struct_literal -= 1;
            let then = self.parse_block()?;
            let else_branch = if self.check_keyword("else") {
                self.bump();
                if self.check_keyword("if") {
                    Some(Box::new(self.parse_prefix_expr()?))
                } else {
                    Some(Box::new(Expr::Block(Box::new(self.parse_block()?))))
                }
            } else {
                None
            };
            return Ok(Expr::If {
                cond: Box::new(cond),
                then: Box::new(then),
                else_branch,
            });
        }
        
        // match expression
        if self.check_keyword("match") {
            self.bump();
            self.no_struct_literal += 1;
            let scrutinee = self.parse_expr()?;
            self.no_struct_literal -= 1;
            self.expect(&TokenKind::LBrace)?;
            let mut arms = Vec::new();
            while !self.check(&TokenKind::RBrace) && !self.at_end() {
                let pattern = self.parse_pattern()?;
                let guard = if self.check_keyword("if") {
                    self.bump();
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                self.expect(&TokenKind::FatArrow)?;
                let body = self.parse_expr()?;
                arms.push(MatchArm { pattern, guard, body });
                if !self.eat(&TokenKind::Comma).is_some() {
                    break;
                }
            }
            self.expect(&TokenKind::RBrace)?;
            return Ok(Expr::Match { scrutinee: Box::new(scrutinee), arms });
        }
        
        // loop expression
        if self.check_keyword("loop") {
            self.bump();
            let body = self.parse_block()?;
            return Ok(Expr::Loop(Box::new(body)));
        }
        
        // while expression
        if self.check_keyword("while") {
            self.bump();
            self.no_struct_literal += 1;
            let cond = self.parse_expr()?;
            self.no_struct_literal -= 1;
            let body = self.parse_block()?;
            return Ok(Expr::While { cond: Box::new(cond), body: Box::new(body) });
        }
        
        // for expression
        if self.check_keyword("for") {
            self.bump();
            let pattern = self.parse_pattern()?;
            self.expect_keyword("in")?;
            self.no_struct_literal += 1;
            let iterable = self.parse_expr()?;
            self.no_struct_literal -= 1;
            let body = self.parse_block()?;
            return Ok(Expr::For {
                pattern: Box::new(pattern),
                iterable: Box::new(iterable),
                body: Box::new(body),
            });
        }
        
        // block expression
        if self.check(&TokenKind::LBrace) {
            let block = self.parse_block()?;
            return Ok(Expr::Block(Box::new(block)));
        }
        
        // unsafe block
        if self.check_keyword("unsafe") {
            self.bump();
            let block = self.parse_block()?;
            return Ok(Expr::Unsafe(Box::new(block)));
        }
        
        // async block
        if self.check_keyword("async") {
            self.bump();
            let block = self.parse_block()?;
            return Ok(Expr::Async(Box::new(block)));
        }
        
        // return
        if self.check_keyword("return") {
            self.bump();
            let value = if self.check_any(&[
                TokenKind::Semi, TokenKind::RBrace, TokenKind::Comma,
                TokenKind::RBracket, TokenKind::RParen, TokenKind::Eof,
            ]) {
                None
            } else {
                Some(Box::new(self.parse_expr()?))
            };
            return Ok(Expr::Return(value));
        }
        
        // break
        if self.check_keyword("break") {
            self.bump();
            // Label (optional)
            let label = if matches!(self.current_kind(), TokenKind::Ident(_)) {
                // Could be a label or a value - simplified
                None
            } else {
                None
            };
            let value = if self.check_any(&[
                TokenKind::Semi, TokenKind::RBrace, TokenKind::Comma,
                TokenKind::RBracket, TokenKind::RParen, TokenKind::Eof,
            ]) {
                None
            } else {
                Some(Box::new(self.parse_expr()?))
            };
            return Ok(Expr::Break { label, value });
        }
        
        // continue
        if self.check_keyword("continue") {
            self.bump();
            return Ok(Expr::Continue { label: None });
        }
        
        // Closure: |args| body
        if self.check(&TokenKind::Or) {
            self.bump();
            let mut params = Vec::new();
            while !self.check(&TokenKind::Or) && !self.at_end() {
                let param = self.parse_closure_param()?;
                params.push(param);
                if !self.eat(&TokenKind::Comma).is_some() {
                    break;
                }
            }
            self.expect(&TokenKind::Or)?;
            let body = self.parse_expr()?;
            return Ok(Expr::Closure { params, body: Box::new(body) });
        }
        
        // box expression
        if self.check_keyword("box") {
            self.bump();
            let expr = self.parse_unary_expr()?;
            return Ok(Expr::Box(Box::new(expr)));
        }
        
        // Parenthesized expression
        if self.check(&TokenKind::LParen) {
            self.bump();
            if self.check(&TokenKind::RParen) {
                self.bump();
                return Ok(Expr::Tuple(vec![]));
            }
            let first = self.parse_expr()?;
            if self.check(&TokenKind::Comma) {
                let mut items = vec![first];
                while self.eat(&TokenKind::Comma).is_some() {
                    if self.check(&TokenKind::RParen) { break; }
                    items.push(self.parse_expr()?);
                }
                self.expect(&TokenKind::RParen)?;
                return Ok(Expr::Tuple(items));
            }
            self.expect(&TokenKind::RParen)?;
            return Ok(first);
        }
        
        // Array literal
        if self.check(&TokenKind::LBracket) {
            self.bump();
            if self.check(&TokenKind::RBracket) {
                self.bump();
                return Ok(Expr::Array(vec![]));
            }
            let first = self.parse_expr()?;
            if self.check(&TokenKind::Semi) {
                self.bump();
                let count = self.parse_expr()?;
                self.expect(&TokenKind::RBracket)?;
                return Ok(Expr::ArrayRepeat {
                    element: Box::new(first),
                    count: Box::new(count),
                });
            }
            let mut items = vec![first];
            while self.eat(&TokenKind::Comma).is_some() {
                if self.check(&TokenKind::RBracket) { break; }
                items.push(self.parse_expr()?);
            }
            self.expect(&TokenKind::RBracket)?;
            return Ok(Expr::Array(items));
        }
        
        // Struct literal: Path { fields }
        // This is handled in postfix, but for prefix we check if it's a path followed by {
        if matches!(self.current_kind(), TokenKind::Ident(_)) {
            // Save position for backtracking
            let save_pos = self.pos;
            if let Ok(path) = self.parse_path() {
                if self.check(&TokenKind::LBrace) {
                    self.bump();
                    let mut fields = Vec::new();
                    while !self.check(&TokenKind::RBrace) && !self.at_end() {
                        let name = self.parse_ident()?;
                        let value = if self.check(&TokenKind::Colon) {
                            self.bump();
                            self.parse_expr()?
                        } else {
                            // Shorthand: field name is also the value
                            Expr::Ident(name.clone())
                        };
                        let shorthand = matches!(value, Expr::Ident(ref id) if id.0 == name.0);
                        fields.push(FieldValue { name, value, shorthand });
                        if !self.eat(&TokenKind::Comma).is_some() {
                            break;
                        }
                    }
                    self.expect(&TokenKind::RBrace)?;
                    return Ok(Expr::StructLit { path, fields });
                }
            }
            // Reset position
            self.pos = save_pos;
        }
        
        // format! macro
        if matches!(self.current_kind(), TokenKind::Ident(_)) {
            let save_pos = self.pos;
            if let TokenKind::Ident(name) = self.current_kind().clone() {
                if name == "format" && self.peek_at(1).value == TokenKind::Not {
                    self.bump(); // format
                    self.bump(); // !
                    self.expect(&TokenKind::LParen)?;
                    let template = if let TokenKind::StrLit(s) = self.bump().value {
                        s
                    } else {
                        String::new()
                    };
                    let mut args = Vec::new();
                    while !self.check(&TokenKind::RParen) && !self.at_end() {
                        if !self.check(&TokenKind::Comma) {
                            args.push(self.parse_expr()?);
                        }
                        if !self.eat(&TokenKind::Comma).is_some() {
                            break;
                        }
                    }
                    self.expect(&TokenKind::RParen)?;
                    return Ok(Expr::Format { template, args });
                }
                if name == "panic" && self.peek_at(1).value == TokenKind::Not {
                    self.bump(); // panic
                    self.bump(); // !
                    self.expect(&TokenKind::LParen)?;
                    let arg = if self.check(&TokenKind::RParen) {
                        None
                    } else {
                        Some(Box::new(self.parse_expr()?))
                    };
                    self.expect(&TokenKind::RParen)?;
                    return Ok(Expr::Panic(arg));
                }
            }
            self.pos = save_pos;
        }
        
        // Tuple struct literal: Path(args)
        // Already handled by parse_path + postfix call
        
        let span = self.peek().span;
        Err(ParseError::new(
            format!("expected expression, found {}", self.current_kind()),
            span,
        ))
    }
    
    /// Finish parsing and return any remaining tokens
    pub fn finish(self) -> Vec<Token> {
        self.tokens[self.pos..].to_vec()
    }
}

/// Convenience function to parse a source string
pub fn parse(source: &str) -> ParseResult<SourceFile> {
    let tokens = crate::lexer::lex(source, crate::span::FileId::new(0));
    let mut parser = Parser::new(tokens);
    parser.parse_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_let() {
        let source = "fn main() { let x = 42; }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::Fn(func) = &file.items[0] {
            assert_eq!(func.name.0, "main");
            assert!(func.body.is_some());
        } else {
            panic!("expected function");
        }
    }
    
    #[test]
    fn test_parse_struct() {
        let source = "struct Point { x: f64, y: f64 }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::Struct(s) = &file.items[0] {
            assert_eq!(s.name.0, "Point");
            assert_eq!(s.fields.len(), 2);
        } else {
            panic!("expected struct");
        }
    }
    
    #[test]
    fn test_parse_enum() {
        let source = "enum Option<T> { Some(T), None }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::Enum(e) = &file.items[0] {
            assert_eq!(e.name.0, "Option");
            assert_eq!(e.variants.len(), 2);
        } else {
            panic!("expected enum");
        }
    }
    
    #[test]
    fn test_parse_fn_call() {
        let source = "fn main() { println(\"hello\"); }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_match() {
        let source = "fn main() { match x { 1 => a, _ => b } }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_if() {
        let source = "fn main() { if x { 1 } else { 2 } }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_for() {
        let source = "fn main() { for i in 0..10 { } }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_binary() {
        let source = "fn main() { let x = 1 + 2 * 3; }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_impl() {
        let source = "impl Point { fn new() -> Self { Self { x: 0.0, y: 0.0 } } }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::Impl(i) = &file.items[0] {
            assert_eq!(i.items.len(), 1);
        } else {
            panic!("expected impl");
        }
    }
    
    #[test]
    fn test_parse_trait() {
        let source = "trait Display { fn fmt(&self) -> String; }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::Trait(t) = &file.items[0] {
            assert_eq!(t.name.0, "Display");
            assert_eq!(t.items.len(), 1);
        } else {
            panic!("expected trait");
        }
    }
    
    #[test]
    fn test_parse_use() {
        let source = "use std::io::{Read, Write};";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::Use(u) = &file.items[0] {
            assert_eq!(u.items.len(), 2);
        } else {
            panic!("expected use");
        }
    }
    
    #[test]
    fn test_parse_generics() {
        let source = "fn foo<T: Clone>(x: T) -> T { x }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::Fn(func) = &file.items[0] {
            assert_eq!(func.generics.len(), 1);
            assert_eq!(func.params.len(), 1);
        } else {
            panic!("expected function");
        }
    }
    
    #[test]
    fn test_parse_closure() {
        let source = "fn main() { let f = |x: i32| x + 1; }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_array() {
        let source = "fn main() { let a = [1, 2, 3]; let b = [0; 10]; }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_tuple_struct() {
        let source = "struct Color(u8, u8, u8);";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::Struct(s) = &file.items[0] {
            assert!(matches!(s.kind, StructKind::Tuple));
            assert_eq!(s.fields.len(), 3);
        } else {
            panic!("expected struct");
        }
    }
    
    #[test]
    fn test_parse_union() {
        let source = "union IntFloat { i: i32, f: f32 }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::Union(u) = &file.items[0] {
            assert_eq!(u.name.0, "IntFloat");
            assert_eq!(u.fields.len(), 2);
        } else {
            panic!("expected union");
        }
    }
    
    #[test]
    fn test_parse_type_alias() {
        let source = "type Result<T> = std::result::Result<T, Error>;";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::TypeAlias { name, .. } = &file.items[0] {
            assert_eq!(name.0, "Result");
        } else {
            panic!("expected type alias");
        }
    }
    
    #[test]
    fn test_parse_const() {
        let source = "const MAX: usize = 1024;";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::Const { name, .. } = &file.items[0] {
            assert_eq!(name.0, "MAX");
        } else {
            panic!("expected const");
        }
    }
    
    #[test]
    fn test_parse_static() {
        let source = "static mut COUNTER: i32 = 0;";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::Static { name, .. } = &file.items[0] {
            assert_eq!(name.0, "COUNTER");
        } else {
            panic!("expected static");
        }
    }
    
    #[test]
    fn test_parse_mod() {
        let source = "mod foo { pub fn bar() {} }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::Mod { name, items } = &file.items[0] {
            assert_eq!(name.0, "foo");
            assert_eq!(items.len(), 1);
        } else {
            panic!("expected mod");
        }
    }
    
    #[test]
    fn test_parse_method_call() {
        let source = "fn main() { x.foo(1, 2); }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_index() {
        let source = "fn main() { let x = a[0]; }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_cast() {
        let source = "fn main() { let x = 1 as i64; }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_reference() {
        let source = "fn main() { let x = &y; let z = &mut y; }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_range() {
        let source = "fn main() { for i in 0..10 { } for j in 0..=10 { } }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_try() {
        let source = "fn main() { let x = foo()?; }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_struct_lit() {
        let source = "fn main() { let p = Point { x: 1.0, y: 2.0 }; }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_return() {
        let source = "fn main() -> i32 { return 42; }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_break() {
        let source = "fn main() { loop { break; } }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_continue() {
        let source = "fn main() { loop { continue; } }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_field_access() {
        let source = "fn main() { let x = p.x; }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_tuple_field() {
        let source = "fn main() { let x = p.0; }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_println_macro() {
        let source = "fn main() { println!(\"Hello, {}!\", name); }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::Fn(func) = &file.items[0] {
            let body = func.body.as_ref().unwrap();
            match &body.stmts[0] {
                Stmt::Expr(Expr::Print { newline, template, args }) => {
                    assert!(*newline);
                    assert_eq!(template, "Hello, {}!");
                    assert_eq!(args.len(), 1);
                }
                other => panic!("expected println!, got {:?}", other),
            }
        } else {
            panic!("expected function");
        }
    }
    
    #[test]
    fn test_parse_print_macro() {
        let source = "fn main() { print!(\"x\"); }";
        let file = parse(source).unwrap();
        if let Item::Fn(func) = &file.items[0] {
            let body = func.body.as_ref().unwrap();
            match &body.stmts[0] {
                Stmt::Expr(Expr::Print { newline, .. }) => assert!(!*newline),
                other => panic!("expected print!, got {:?}", other),
            }
        }
    }
    
    #[test]
    fn test_parse_vec_macro() {
        let source = "fn main() { let v = vec![1, 2, 3]; }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_panic_macro() {
        let source = "fn main() { panic!(\"boom\"); }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }
    
    #[test]
    fn test_parse_self_struct_literal() {
        let source = "impl Point { fn new() -> Self { Self { x: 0.0, y: 0.0 } } }";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
    }

    #[test]
    fn test_parse_leading_comments() {
        let source = "// a line comment\n/* block */\nfn main() { /* inline */ let x = 1; }\n// trailing";
        let file = parse(source).unwrap();
        assert_eq!(file.items.len(), 1);
        if let Item::Fn(func) = &file.items[0] {
            assert_eq!(func.name.0, "main");
            assert_eq!(func.body.as_ref().unwrap().stmts.len(), 1);
        } else {
            panic!("expected function");
        }
    }
}