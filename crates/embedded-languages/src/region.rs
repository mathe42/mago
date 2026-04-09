use mago_span::Span;

use crate::mapping::PositionMapping;

/// The embedded language detected in a PHP string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddedLanguage {
    Sql,
    Bash,
}

/// How confident is the detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionConfidence {
    /// Detected via function/method context (e.g., `$pdo->query("...")`).
    Strong,
    /// Detected via string content heuristic (e.g., starts with SELECT).
    Heuristic,
    /// Detected via heredoc/nowdoc label (e.g., `<<<SQL`).
    Label,
}

/// A detected region of embedded language content within a PHP file.
#[derive(Debug, Clone)]
pub struct EmbeddedRegion {
    /// The language detected.
    pub language: EmbeddedLanguage,
    /// The span in the original PHP file covering the string expression.
    pub php_span: Span,
    /// The confidence of the detection.
    pub confidence: DetectionConfidence,
    /// The extracted virtual document content (with placeholders for interpolations).
    pub virtual_document: String,
    /// The position mapping between virtual document and PHP file offsets.
    pub mapping: PositionMapping,
}
