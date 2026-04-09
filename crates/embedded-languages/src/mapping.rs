use std::ops::Range;

use mago_span::Span;

/// Maps positions between a virtual embedded document and the original PHP source.
#[derive(Debug, Clone)]
pub struct PositionMapping {
    /// Ordered segments that map virtual document ranges to PHP source ranges.
    pub segments: Vec<MappingSegment>,
}

/// A segment in the position mapping.
#[derive(Debug, Clone)]
pub enum MappingSegment {
    /// Literal text — virtual offset maps 1:1 to PHP offset.
    Literal {
        virtual_range: Range<u32>,
        php_range: Range<u32>,
    },
    /// Placeholder replacing a PHP expression (e.g., `$var` in interpolated string).
    /// No meaningful mapping into the embedded language exists here.
    Placeholder {
        virtual_range: Range<u32>,
        php_span: Span,
    },
}

impl PositionMapping {
    /// Create a new empty mapping.
    pub fn new() -> Self {
        Self { segments: Vec::new() }
    }

    /// Create a simple 1:1 mapping for a non-interpolated string.
    ///
    /// `content_start` is the byte offset in the PHP file where the string content begins
    /// (after the opening quote).
    pub fn simple(content_len: u32, content_start: u32) -> Self {
        Self {
            segments: vec![MappingSegment::Literal {
                virtual_range: 0..content_len,
                php_range: content_start..content_start + content_len,
            }],
        }
    }

    /// Map a virtual document byte offset to a PHP source byte offset.
    ///
    /// Returns `None` if the offset falls within a placeholder region.
    pub fn virtual_to_php(&self, virtual_offset: u32) -> Option<u32> {
        for segment in &self.segments {
            match segment {
                MappingSegment::Literal { virtual_range, php_range } => {
                    if virtual_range.contains(&virtual_offset) {
                        let delta = virtual_offset - virtual_range.start;
                        return Some(php_range.start + delta);
                    }
                }
                MappingSegment::Placeholder { virtual_range, .. } => {
                    if virtual_range.contains(&virtual_offset) {
                        return None; // Inside a placeholder
                    }
                }
            }
        }
        None
    }

    /// Map a PHP source byte offset to a virtual document byte offset.
    ///
    /// Returns `None` if the offset doesn't fall within any mapped region.
    pub fn php_to_virtual(&self, php_offset: u32) -> Option<u32> {
        for segment in &self.segments {
            if let MappingSegment::Literal { virtual_range, php_range } = segment {
                if php_range.contains(&php_offset) {
                    let delta = php_offset - php_range.start;
                    return Some(virtual_range.start + delta);
                }
            }
        }
        None
    }
}

impl Default for PositionMapping {
    fn default() -> Self {
        Self::new()
    }
}
