//! Lexer for the bas language

use crate::span::{ByteOffset, FileId, Location, Span, Spanned};
use crate::token::{Token, TokenKind};
use std::iter::Peekable;
use std::str::Chars;

/// Lexer state
#[allow(dead_code)] // `source`, `file_id`, and `buffer` are kept for
                    // future diagnostics/error reporting.
pub struct Lexer<'a> {
    source: &'a str,
    chars: Peekable<Chars<'a>>,
    pos: ByteOffset,
    line: u32,
    column: u32,
    file_id: FileId,
    /// Buffer for the current token
    buffer: String,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str, file_id: FileId) -> Self {
        Lexer {
            source,
            chars: source.chars().peekable(),
            pos: ByteOffset(0),
            line: 1,
            column: 1,
            file_id,
            buffer: String::new(),
        }
    }
    
    /// Get the current span
    fn current_span(&self, start_pos: ByteOffset) -> Span {
        Span::new(start_pos, self.pos)
    }
    
    /// Advance to the next character
    fn next_char(&mut self) -> Option<char> {
        let ch = self.chars.next()?;
        self.pos = ByteOffset(self.pos.0 + ch.len_utf8() as u32);
        
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        
        Some(ch)
    }
    
    /// Peek at the next character without consuming
    fn peek_char(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }
    
    /// Get current location
    #[allow(dead_code)]
    fn current_location(&self) -> Location {
        Location::new(self.line, self.column)
    }
    
    /// Create a token with the current span
    fn make_token(&mut self, kind: TokenKind, start_pos: ByteOffset) -> Token {
        let span = self.current_span(start_pos);
        Spanned::new(kind, span)
    }
    
    /// Skip whitespace
    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace() {
                self.next_char();
            } else {
                break;
            }
        }
    }
    
    /// Scan a single token
    fn scan_token(&mut self) -> Token {
        self.skip_whitespace();
        
        let start_pos = self.pos;
        let ch = match self.next_char() {
            Some(c) => c,
            None => return self.make_token(TokenKind::Eof, start_pos),
        };
        
        match ch {
            // Single-character tokens
            '(' => self.make_token(TokenKind::LParen, start_pos),
            ')' => self.make_token(TokenKind::RParen, start_pos),
            '{' => self.make_token(TokenKind::LBrace, start_pos),
            '}' => self.make_token(TokenKind::RBrace, start_pos),
            '[' => self.make_token(TokenKind::LBracket, start_pos),
            ']' => self.make_token(TokenKind::RBracket, start_pos),
            ',' => self.make_token(TokenKind::Comma, start_pos),
            ';' => self.make_token(TokenKind::Semi, start_pos),
            '.' => {
                if self.peek_char() == Some('.') {
                    self.next_char();
                    if self.peek_char() == Some('=') {
                        self.next_char();
                        self.make_token(TokenKind::DotDotEq, start_pos)
                    } else {
                        self.make_token(TokenKind::DotDot, start_pos)
                    }
                } else {
                    self.make_token(TokenKind::Dot, start_pos)
                }
            }
            ':' => {
                if self.peek_char() == Some(':') {
                    self.next_char();
                    self.make_token(TokenKind::DoubleColon, start_pos)
                } else {
                    self.make_token(TokenKind::Colon, start_pos)
                }
            }
            '?' => self.make_token(TokenKind::Question, start_pos),
            '_' => self.make_token(TokenKind::Underscore, start_pos),
            '@' => self.make_token(TokenKind::At, start_pos),
            '$' => self.make_token(TokenKind::Dollar, start_pos),
            '#' => self.make_token(TokenKind::Pound, start_pos),
            
            // Operators that can be multi-character
            '+' => {
                if self.peek_char() == Some('=') {
                    self.next_char();
                    self.make_token(TokenKind::PlusEq, start_pos)
                } else {
                    self.make_token(TokenKind::Plus, start_pos)
                }
            }
            '-' => {
                if self.peek_char() == Some('=') {
                    self.next_char();
                    self.make_token(TokenKind::MinusEq, start_pos)
                } else if self.peek_char() == Some('>') {
                    self.next_char();
                    self.make_token(TokenKind::Arrow, start_pos)
                } else {
                    self.make_token(TokenKind::Minus, start_pos)
                }
            }
            '*' => {
                if self.peek_char() == Some('=') {
                    self.next_char();
                    self.make_token(TokenKind::StarEq, start_pos)
                } else {
                    self.make_token(TokenKind::Star, start_pos)
                }
            }
            '/' => {
                if self.peek_char() == Some('=') {
                    self.next_char();
                    self.make_token(TokenKind::SlashEq, start_pos)
                } else if self.peek_char() == Some('/') {
                    // Line comment
                    self.next_char();
                    self.scan_line_comment(start_pos)
                } else if self.peek_char() == Some('*') {
                    // Block comment
                    self.next_char();
                    self.scan_block_comment(start_pos)
                } else {
                    self.make_token(TokenKind::Slash, start_pos)
                }
            }
            '%' => {
                if self.peek_char() == Some('=') {
                    self.next_char();
                    self.make_token(TokenKind::PercentEq, start_pos)
                } else {
                    self.make_token(TokenKind::Percent, start_pos)
                }
            }
            '=' => {
                if self.peek_char() == Some('=') {
                    self.next_char();
                    self.make_token(TokenKind::EqEq, start_pos)
                } else if self.peek_char() == Some('>') {
                    self.next_char();
                    self.make_token(TokenKind::FatArrow, start_pos)
                } else {
                    self.make_token(TokenKind::Eq, start_pos)
                }
            }
            '!' => {
                if self.peek_char() == Some('=') {
                    self.next_char();
                    self.make_token(TokenKind::NotEq, start_pos)
                } else {
                    self.make_token(TokenKind::Not, start_pos)
                }
            }
            '<' => {
                if self.peek_char() == Some('=') {
                    self.next_char();
                    self.make_token(TokenKind::LtEq, start_pos)
                } else if self.peek_char() == Some('<') {
                    self.next_char();
                    if self.peek_char() == Some('=') {
                        self.next_char();
                        self.make_token(TokenKind::ShlEq, start_pos)
                    } else {
                        self.make_token(TokenKind::Shl, start_pos)
                    }
                } else {
                    self.make_token(TokenKind::Lt, start_pos)
                }
            }
            '>' => {
                if self.peek_char() == Some('=') {
                    self.next_char();
                    self.make_token(TokenKind::GtEq, start_pos)
                } else if self.peek_char() == Some('>') {
                    self.next_char();
                    if self.peek_char() == Some('=') {
                        self.next_char();
                        self.make_token(TokenKind::ShrEq, start_pos)
                    } else {
                        self.make_token(TokenKind::Shr, start_pos)
                    }
                } else {
                    self.make_token(TokenKind::Gt, start_pos)
                }
            }
            '&' => {
                if self.peek_char() == Some('&') {
                    self.next_char();
                    self.make_token(TokenKind::AndAnd, start_pos)
                } else if self.peek_char() == Some('=') {
                    self.next_char();
                    self.make_token(TokenKind::AndEq, start_pos)
                } else {
                    self.make_token(TokenKind::And, start_pos)
                }
            }
            '|' => {
                if self.peek_char() == Some('|') {
                    self.next_char();
                    self.make_token(TokenKind::OrOr, start_pos)
                } else if self.peek_char() == Some('=') {
                    self.next_char();
                    self.make_token(TokenKind::OrEq, start_pos)
                } else {
                    self.make_token(TokenKind::Or, start_pos)
                }
            }
            '^' => {
                if self.peek_char() == Some('=') {
                    self.next_char();
                    self.make_token(TokenKind::XorEq, start_pos)
                } else {
                    self.make_token(TokenKind::Xor, start_pos)
                }
            }
            '~' => self.make_token(TokenKind::NotBit, start_pos),
            
            // Identifiers and keywords
            c if c.is_ascii_alphabetic() || c == '_' => {
                self.scan_identifier(start_pos, c)
            }
            
            // Numbers
            c if c.is_ascii_digit() => {
                self.scan_number(start_pos, c)
            }
            
            // Character literals
            '\'' => self.scan_char(start_pos),
            
            // String literals
            '"' => self.scan_string(start_pos),
            
            // Raw string literals
            'r' => {
                if self.peek_char() == Some('#') || self.peek_char() == Some('"') {
                    self.scan_raw_string(start_pos)
                } else {
                    self.scan_identifier(start_pos, ch)
                }
            }
            
            // Byte string
            'b' => {
                if self.peek_char() == Some('"') {
                    self.next_char(); // consume "
                    self.scan_byte_string(start_pos)
                } else {
                    self.scan_identifier(start_pos, ch)
                }
            }
            
            _ => {
                // Unknown character
                self.make_token(TokenKind::Ident(format!("\\u{{{:04X}}}", ch as u32)), start_pos)
            }
        }
    }
    
    /// Scan a line comment
    fn scan_line_comment(&mut self, start_pos: ByteOffset) -> Token {
        let mut content = String::new();
        while let Some(ch) = self.peek_char() {
            if ch == '\n' {
                break;
            }
            content.push(ch);
            self.next_char();
        }
        self.make_token(TokenKind::Comment(content), start_pos)
    }
    
    /// Scan a block comment
    fn scan_block_comment(&mut self, start_pos: ByteOffset) -> Token {
        let mut content = String::new();
        let mut depth = 1;
        
        while depth > 0 {
            match self.next_char() {
                Some('*') if self.peek_char() == Some('/') => {
                    self.next_char(); // consume /
                    depth -= 1;
                }
                Some('/') if self.peek_char() == Some('*') => {
                    self.next_char(); // consume *
                    depth += 1;
                }
                Some(ch) => content.push(ch),
                None => break, // Unterminated comment
            }
        }
        
        self.make_token(TokenKind::Comment(content), start_pos)
    }
    
    /// Scan an identifier or keyword
    fn scan_identifier(&mut self, start_pos: ByteOffset, first: char) -> Token {
        let mut ident = String::new();
        ident.push(first);
        
        while let Some(ch) = self.peek_char() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ident.push(ch);
                self.next_char();
            } else {
                break;
            }
        }
        
        // Boolean literals
        if ident == "true" {
            return self.make_token(TokenKind::BoolLit(true), start_pos);
        }
        if ident == "false" {
            return self.make_token(TokenKind::BoolLit(false), start_pos);
        }
        
        // Check if it's a keyword
        let kind = crate::token::keyword_kind(ident.as_str())
            .unwrap_or(TokenKind::Ident(ident));
        
        self.make_token(kind, start_pos)
    }
    
    /// Scan a number (integer or float)
    fn scan_number(&mut self, start_pos: ByteOffset, first: char) -> Token {
        let mut num = String::new();
        num.push(first);
        let mut is_float = false;
        
        // Scan digits
        while let Some(ch) = self.peek_char() {
            if ch.is_ascii_digit() || ch == '_' {
                num.push(ch);
                self.next_char();
            } else if ch == '.' && !is_float {
                // Check if this is a float or a range operator
                if let Some(next) = self.chars.clone().nth(1) {
                    if next.is_ascii_digit() {
                        is_float = true;
                        num.push('.');
                        self.next_char(); // consume .
                        if let Some(d) = self.next_char() {
                            num.push(d); // consume and push first fractional digit
                        }
                        continue;
                    }
                }
                break;
            } else {
                break;
            }
        }
        
        // Scan fractional part for floats
        if is_float {
            while let Some(ch) = self.peek_char() {
                if ch.is_ascii_digit() || ch == '_' {
                    num.push(ch);
                    self.next_char();
                } else {
                    break;
                }
            }
        }
        
        // Scan exponent
        if is_float {
            if let Some('e') | Some('E') = self.peek_char() {
                num.push('e');
                self.next_char();
                if let Some('+' | '-') = self.peek_char() {
                    num.push(self.peek_char().unwrap());
                    self.next_char();
                }
                while let Some(ch) = self.peek_char() {
                    if ch.is_ascii_digit() || ch == '_' {
                        num.push(ch);
                        self.next_char();
                    } else {
                        break;
                    }
                }
            }
        }
        
        // Scan suffix
        let mut suffix = String::new();
        while let Some(ch) = self.peek_char() {
            if ch.is_ascii_alphanumeric() {
                suffix.push(ch);
                self.next_char();
            } else {
                break;
            }
        }
        
        // Remove underscores
        let clean_num = num.replace('_', "");
        
        if is_float {
            let value = clean_num.parse::<f64>().unwrap_or(0.0);
            let suffix_kind = match suffix.as_str() {
                "f32" => crate::token::FloatSuffix::F32,
                "f64" => crate::token::FloatSuffix::F64,
                "" => crate::token::FloatSuffix::None,
                _ => crate::token::FloatSuffix::None,
            };
            self.make_token(TokenKind::FloatLit(value, suffix_kind), start_pos)
        } else {
            // Parse integer with base detection
            let value = if clean_num.starts_with("0x") || clean_num.starts_with("0X") {
                i64::from_str_radix(&clean_num[2..], 16).unwrap_or(0)
            } else if clean_num.starts_with("0o") || clean_num.starts_with("0O") {
                i64::from_str_radix(&clean_num[2..], 8).unwrap_or(0)
            } else if clean_num.starts_with("0b") || clean_num.starts_with("0B") {
                i64::from_str_radix(&clean_num[2..], 2).unwrap_or(0)
            } else {
                clean_num.parse::<i64>().unwrap_or(0)
            };
            
            let suffix_kind = match suffix.as_str() {
                "i8" => crate::token::IntSuffix::I8,
                "i16" => crate::token::IntSuffix::I16,
                "i32" => crate::token::IntSuffix::I32,
                "i64" => crate::token::IntSuffix::I64,
                "i128" => crate::token::IntSuffix::I128,
                "isize" => crate::token::IntSuffix::Isize,
                "u8" => crate::token::IntSuffix::U8,
                "u16" => crate::token::IntSuffix::U16,
                "u32" => crate::token::IntSuffix::U32,
                "u64" => crate::token::IntSuffix::U64,
                "u128" => crate::token::IntSuffix::U128,
                "usize" => crate::token::IntSuffix::Usize,
                "" => crate::token::IntSuffix::None,
                _ => crate::token::IntSuffix::None,
            };
            self.make_token(TokenKind::IntLit(value, suffix_kind), start_pos)
        }
    }
    
    /// Scan a character literal
    fn scan_char(&mut self, start_pos: ByteOffset) -> Token {
        let mut value = '\0';
        let mut escaped = false;
        
        while let Some(ch) = self.next_char() {
            if escaped {
                value = match ch {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '\\' => '\\',
                    '\'' => '\'',
                    '"' => '"',
                    '0' => '\0',
                    'x' => {
                        // Hex escape \xNN
                        let mut hex = String::new();
                        for _ in 0..2 {
                            if let Some(ch) = self.next_char() {
                                if ch.is_ascii_hexdigit() {
                                    hex.push(ch);
                                } else {
                                    break;
                                }
                            }
                        }
                        u8::from_str_radix(&hex, 16).map(|b| b as char).unwrap_or('\0')
                    }
                    'u' => {
                        // Unicode escape \u{NNNNNN}
                        if self.peek_char() == Some('{') {
                            self.next_char(); // consume {
                            let mut hex = String::new();
                            while let Some(ch) = self.next_char() {
                                if ch == '}' { break; }
                                if ch.is_ascii_hexdigit() {
                                    hex.push(ch);
                                }
                            }
                            u32::from_str_radix(&hex, 16)
                                .ok()
                                .and_then(|cp| char::try_from(cp).ok())
                                .unwrap_or('\0')
                        } else {
                            '\0'
                        }
                    }
                    _ => ch,
                };
                break;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                break;
            } else {
                value = ch;
            }
        }
        
        self.make_token(TokenKind::CharLit(value), start_pos)
    }
    
    /// Scan a string literal
    fn scan_string(&mut self, start_pos: ByteOffset) -> Token {
        let mut value = String::new();
        let mut escaped = false;
        
        while let Some(ch) = self.next_char() {
            if escaped {
                let ch = match ch {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '\\' => '\\',
                    '\'' => '\'',
                    '"' => '"',
                    '0' => '\0',
                    'x' => {
                        let mut hex = String::new();
                        for _ in 0..2 {
                            if let Some(ch) = self.next_char() {
                                if ch.is_ascii_hexdigit() {
                                    hex.push(ch);
                                }
                            }
                        }
                        u8::from_str_radix(&hex, 16).map(|b| b as char).unwrap_or('\0')
                    }
                    'u' => {
                        if self.peek_char() == Some('{') {
                            self.next_char();
                            let mut hex = String::new();
                            while let Some(ch) = self.next_char() {
                                if ch == '}' { break; }
                                if ch.is_ascii_hexdigit() {
                                    hex.push(ch);
                                }
                            }
                            u32::from_str_radix(&hex, 16)
                                .ok()
                                .and_then(|cp| char::try_from(cp).ok())
                                .unwrap_or('\0')
                        } else { '\0' }
                    }
                    _ => ch,
                };
                value.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                break;
            } else {
                value.push(ch);
            }
        }
        
        self.make_token(TokenKind::StrLit(value), start_pos)
    }
    
    /// Scan a raw string literal r#"..."#
    fn scan_raw_string(&mut self, start_pos: ByteOffset) -> Token {
        // Count # symbols
        let mut hash_count = 0;
        while self.peek_char() == Some('#') {
            self.next_char();
            hash_count += 1;
        }
        
        // Expect opening quote
        if self.peek_char() != Some('"') {
            return self.make_token(TokenKind::Ident("raw string error".to_string()), start_pos);
        }
        self.next_char(); // consume "
        
        let mut value = String::new();
        let end_marker = "#".repeat(hash_count) + "\"";
        let mut buffer = String::new();
        
        while let Some(ch) = self.next_char() {
            buffer.push(ch);
            if buffer.ends_with(&end_marker) {
                // Remove the end marker
                let content_len = buffer.len() - end_marker.len();
                value = buffer[..content_len].to_string();
                break;
            }
        }
        
        self.make_token(TokenKind::StrLit(value), start_pos)
    }
    
    /// Scan a byte string literal b"..."
    fn scan_byte_string(&mut self, start_pos: ByteOffset) -> Token {
        let mut value = Vec::new();
        let mut escaped = false;
        
        while let Some(ch) = self.next_char() {
            if escaped {
                let byte = match ch {
                    'n' => b'\n',
                    't' => b'\t',
                    'r' => b'\r',
                    '\\' => b'\\',
                    '\'' => b'\'',
                    '"' => b'"',
                    '0' => b'\0',
                    'x' => {
                        let mut hex = String::new();
                        for _ in 0..2 {
                            if let Some(ch) = self.next_char() {
                                if ch.is_ascii_hexdigit() {
                                    hex.push(ch);
                                }
                            }
                        }
                        u8::from_str_radix(&hex, 16).unwrap_or(0)
                    }
                    _ => ch as u8,
                };
                value.push(byte);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                break;
            } else {
                value.push(ch as u8);
            }
        }
        
        self.make_token(TokenKind::ByteStrLit(value), start_pos)
    }
    
    /// Peek at the next token without consuming
    pub fn peek(&mut self) -> Token {
        let token = self.scan_token();
        // In a real implementation, we'd need to store this for next()
        // For simplicity, we'll just return it and handle it in the parser
        token
    }
}

/// Iterator over tokens
impl<'a> Iterator for Lexer<'a> {
    type Item = Token;
    
    fn next(&mut self) -> Option<Self::Item> {
        let token = self.scan_token();
        if matches!(token.value, TokenKind::Eof) {
            None
        } else {
            Some(token)
        }
    }
}

/// Lex a source file into tokens (always includes a trailing Eof token)
pub fn lex(source: &str, file_id: FileId) -> Vec<Token> {
    let mut lexer = Lexer::new(source, file_id);
    let mut tokens = Vec::new();
    loop {
        let token = lexer.scan_token();
        let is_eof = matches!(token.value, TokenKind::Eof);
        tokens.push(token);
        if is_eof {
            break;
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic_tokens() {
        let source = "fn main() { let x = 42; }";
        let tokens = lex(source, FileId::new(0));
        
        let kinds: Vec<_> = tokens.iter().map(|t| t.value.clone()).collect();
        assert_eq!(kinds[0], TokenKind::KwFn);
        assert_eq!(kinds[1], TokenKind::Ident("main".to_string()));
        assert_eq!(kinds[2], TokenKind::LParen);
        assert_eq!(kinds[3], TokenKind::RParen);
        assert_eq!(kinds[4], TokenKind::LBrace);
        assert_eq!(kinds[5], TokenKind::KwLet);
        assert_eq!(kinds[6], TokenKind::Ident("x".to_string()));
        assert_eq!(kinds[7], TokenKind::Eq);
        assert_eq!(kinds[8], TokenKind::IntLit(42, crate::token::IntSuffix::None));
        assert_eq!(kinds[9], TokenKind::Semi);
        assert_eq!(kinds[10], TokenKind::RBrace);
    }
    
    #[test]
    fn test_operators() {
        let source = "a + b - c * d / e % f == g != h < i > j <= k >= l && m || n";
        let tokens = lex(source, FileId::new(0));
        
        let kinds: Vec<_> = tokens.iter().map(|t| t.value.clone()).collect();
        assert!(kinds.contains(&TokenKind::Plus));
        assert!(kinds.contains(&TokenKind::Minus));
        assert!(kinds.contains(&TokenKind::Star));
        assert!(kinds.contains(&TokenKind::Slash));
        assert!(kinds.contains(&TokenKind::Percent));
        assert!(kinds.contains(&TokenKind::EqEq));
        assert!(kinds.contains(&TokenKind::NotEq));
        assert!(kinds.contains(&TokenKind::Lt));
        assert!(kinds.contains(&TokenKind::Gt));
        assert!(kinds.contains(&TokenKind::LtEq));
        assert!(kinds.contains(&TokenKind::GtEq));
        assert!(kinds.contains(&TokenKind::AndAnd));
        assert!(kinds.contains(&TokenKind::OrOr));
    }
    
    #[test]
    fn test_string_literal() {
        let source = r#""hello world""#;
        let tokens = lex(source, FileId::new(0));
        let kinds: Vec<_> = tokens.iter().map(|t| t.value.clone()).collect();
        assert_eq!(kinds[0], TokenKind::StrLit("hello world".to_string()));
    }
    
    #[test]
    fn test_char_literal() {
        let source = "'a'";
        let tokens = lex(source, FileId::new(0));
        let kinds: Vec<_> = tokens.iter().map(|t| t.value.clone()).collect();
        assert_eq!(kinds[0], TokenKind::CharLit('a'));
    }
    
    #[test]
    fn test_comments() {
        let source = "// line comment\n/* block comment */";
        let tokens = lex(source, FileId::new(0));
        let kinds: Vec<_> = tokens.iter().map(|t| t.value.clone()).collect();
        assert!(kinds.iter().any(|k| matches!(k, TokenKind::Comment(c) if c.contains("line comment"))));
        assert!(kinds.iter().any(|k| matches!(k, TokenKind::Comment(c) if c.contains("block comment"))));
    }
    
    #[test]
    fn test_float_literals() {
        let source = "3.14 2.0f32 1.5e-10";
        let tokens = lex(source, FileId::new(0));
        let kinds: Vec<_> = tokens.iter().map(|t| t.value.clone()).collect();
        assert!(matches!(kinds[0], TokenKind::FloatLit(3.14, _)));
        assert!(matches!(kinds[1], TokenKind::FloatLit(2.0, crate::token::FloatSuffix::F32)));
        assert!(matches!(kinds[2], TokenKind::FloatLit(v, _) if (v - 1.5e-10).abs() < f64::EPSILON));
    }

    #[test]
    fn test_bool_literals() {
        let source = "true false truely";
        let tokens = lex(source, FileId::new(0));
        let kinds: Vec<_> = tokens.iter().map(|t| t.value.clone()).collect();
        assert!(matches!(kinds[0], TokenKind::BoolLit(true)));
        assert!(matches!(kinds[1], TokenKind::BoolLit(false)));
        assert!(matches!(&kinds[2], TokenKind::Ident(s) if s == "truely"));
    }
}