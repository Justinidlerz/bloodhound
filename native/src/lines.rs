//! Byte offset to line/column conversion.
//!
//! The previous implementation rescanned the whole source for every span,
//! which dominated scan time on large bundles. `LineIndex` precomputes line
//! starts once per resource and resolves each offset with a binary search
//! and UTF-16 checkpoints. Even multi-megabyte minified lines need at most
//! 256 bytes of decoding per lookup.

use crate::model::{Location, Span};

/// Precomputed line start offsets for one source text.
pub struct LineIndex<'a> {
    source: &'a str,
    /// Byte offset of the first character of each line. `starts[0] == 0`.
    starts: Vec<u32>,
    checkpoints: Vec<(usize, u32)>,
}

impl<'a> LineIndex<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut starts = vec![0u32];
        let mut checkpoints = vec![(0, 0)];
        let mut units = 0;
        let mut chars = source.char_indices().peekable();
        while let Some((i, ch)) = chars.next() {
            if i - checkpoints.last().unwrap().0 >= 256 {
                checkpoints.push((i, units));
            }
            units += ch.len_utf16() as u32;
            if matches!(ch, '\n' | '\u{2028}' | '\u{2029}')
                || (ch == '\r' && !chars.peek().is_some_and(|(_, c)| *c == '\n'))
            {
                starts.push((i + ch.len_utf8()) as u32);
            }
        }
        Self {
            source,
            starts,
            checkpoints,
        }
    }

    fn units(&self, offset: usize) -> u32 {
        let checkpoint = self
            .checkpoints
            .partition_point(|&(byte, _)| byte <= offset)
            - 1;
        let (byte, units) = self.checkpoints[checkpoint];
        units + self.source[byte..offset].encode_utf16().count() as u32
    }

    /// Converts a byte offset into (1-based line, 0-based UTF-16 column).
    /// Offsets past the end clamp to the end of the text; offsets inside a
    /// multi-byte character round down to the character start.
    pub fn position(&self, offset: u32) -> (u32, u32) {
        let mut offset = (offset as usize).min(self.source.len());
        while !self.source.is_char_boundary(offset) {
            offset -= 1;
        }
        // partition_point yields the first line whose start is > offset.
        let line = self.starts.partition_point(|&s| s as usize <= offset);
        let line_start = self.starts[line - 1] as usize;
        let column = self.units(offset) - self.units(line_start);
        (line as u32, column)
    }

    /// Converts a span into a full location.
    pub fn location(&self, span: Span) -> Location {
        let (line, column) = self.position(span.start);
        let (end_line, end_column) = self.position(span.end.max(span.start));
        Location {
            line,
            column,
            end_line,
            end_column,
            original: None,
        }
    }

    /// Returns a trimmed single-line excerpt of the span for display,
    /// capped at `max` characters.
    pub fn snippet(&self, span: Span, max: usize) -> String {
        let start = (span.start as usize).min(self.source.len());
        let end = (span.end as usize).clamp(start, self.source.len());
        let mut start = start;
        let mut end = end;
        while !self.source.is_char_boundary(start) {
            start -= 1;
        }
        while !self.source.is_char_boundary(end) {
            end += 1;
        }
        let mut result = String::new();
        let mut count = 0;
        let mut space = false;
        for ch in self.source[start..end].chars() {
            if ch.is_whitespace() {
                space = count > 0;
                continue;
            }
            if space {
                if count == max {
                    result.push('…');
                    break;
                }
                result.push(' ');
                count += 1;
                space = false;
            }
            if count == max {
                result.push('…');
                break;
            }
            result.push(ch);
            count += 1;
        }
        result
    }
}

#[cfg(test)]
#[path = "tests/lines.rs"]
mod tests;
