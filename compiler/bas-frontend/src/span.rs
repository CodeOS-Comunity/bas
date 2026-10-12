//! Source span and location tracking

use std::fmt;
use std::ops::Range;

/// A byte offset in the source file
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ByteOffset(pub u32);

impl ByteOffset {
    pub fn new(offset: u32) -> Self {
        ByteOffset(offset)
    }
    
    pub fn to_usize(&self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for ByteOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A source code location (line, column)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Location {
    pub line: u32,   // 1-indexed
    pub column: u32, // 1-indexed
}

impl Location {
    pub fn new(line: u32, column: u32) -> Self {
        Location { line, column }
    }
    
    pub fn start() -> Self {
        Location { line: 1, column: 1 }
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// A span in the source code (start and end byte offsets)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Span {
    pub start: ByteOffset,
    pub end: ByteOffset,
}

impl Span {
    pub fn new(start: ByteOffset, end: ByteOffset) -> Self {
        Span { start, end }
    }
    
    pub fn dummy() -> Self {
        Span { start: ByteOffset(0), end: ByteOffset(0) }
    }
    
    pub fn is_dummy(&self) -> bool {
        self.start == ByteOffset(0) && self.end == ByteOffset(0)
    }
    
    pub fn len(&self) -> u32 {
        self.end.0 - self.start.0
    }
    
    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
    
    pub fn to_range(&self) -> Range<usize> {
        self.start.to_usize()..self.end.to_usize()
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// A spanned value - a value with an associated source span
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(value: T, span: Span) -> Self {
        Spanned { value, span }
    }
    
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Spanned<U> {
        Spanned {
            value: f(self.value),
            span: self.span,
        }
    }
    
    pub fn as_ref(&self) -> Spanned<&T> {
        Spanned {
            value: &self.value,
            span: self.span,
        }
    }
}

impl<T: fmt::Display> fmt::Display for Spanned<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} @ {}", self.value, self.span)
    }
}

/// A source file identifier
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FileId(pub u32);

impl FileId {
    pub fn new(id: u32) -> Self {
        FileId(id)
    }
    
    pub fn dummy() -> Self {
        FileId(u32::MAX)
    }
    
    pub fn is_dummy(&self) -> bool {
        self.0 == u32::MAX
    }
}

/// A spanned value with file ID
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FileSpan {
    pub file: FileId,
    pub span: Span,
}

impl FileSpan {
    pub fn new(file: FileId, span: Span) -> Self {
        FileSpan { file, span }
    }
    
    pub fn dummy() -> Self {
        FileSpan { file: FileId::dummy(), span: Span::dummy() }
    }
}

/// Helper trait for types that have a span
pub trait SpannedItem {
    fn span(&self) -> Span;
}

impl<T> SpannedItem for Spanned<T> {
    fn span(&self) -> Span {
        self.span
    }
}