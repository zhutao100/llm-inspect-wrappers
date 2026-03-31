use crate::common::{escape_field, Config};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Plain,
    Long,
    Json,
    Base64,
    Binary,
}

impl LineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LineKind::Plain => "plain",
            LineKind::Long => "long",
            LineKind::Json => "json",
            LineKind::Base64 => "base64",
            LineKind::Binary => "binary",
        }
    }
}

fn is_base64ish_byte(b: u8) -> bool {
    matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' | b'=' | b'_' | b'-')
}

fn trim_ascii_ws(mut s: &[u8]) -> &[u8] {
    while let Some((first, rest)) = s.split_first() {
        if first.is_ascii_whitespace() {
            s = rest;
        } else {
            break;
        }
    }
    while let Some((last, rest)) = s.split_last() {
        if last.is_ascii_whitespace() {
            s = rest;
        } else {
            break;
        }
    }
    s
}

pub fn classify_line(raw: &[u8], cfg: &Config) -> LineKind {
    let trimmed = trim_ascii_ws(raw);

    if std::str::from_utf8(raw).is_err() {
        return LineKind::Binary;
    }

    if raw.len() >= cfg.soft_line_chars {
        if matches!(trimmed.first().copied(), Some(b'{') | Some(b'[')) {
            return LineKind::Json;
        }

        if !trimmed.contains(&b' ') {
            let head = &trimmed[..trimmed.len().min(4096)];
            if head.iter().all(|b| is_base64ish_byte(*b)) {
                return LineKind::Base64;
            }
        }
    }

    LineKind::Plain
}

pub fn should_gate_line(raw: &[u8], cfg: &Config) -> (bool, LineKind) {
    let kind = classify_line(raw, cfg);

    if raw.len() > cfg.hard_line_chars {
        return (true, if kind == LineKind::Plain { LineKind::Long } else { kind });
    }

    if raw.len() > cfg.soft_line_chars && kind != LineKind::Plain {
        return (true, kind);
    }

    (false, kind)
}

fn safe_preview_bytes(raw: &[u8], max_bytes: usize) -> String {
    let head = &raw[..raw.len().min(max_bytes)];
    let s = String::from_utf8_lossy(head);
    escape_field(&s)
}

fn trim_line_endings(raw: &[u8]) -> &[u8] {
    let mut end = raw.len();
    while end > 0 && (raw[end - 1] == b'\n' || raw[end - 1] == b'\r') {
        end -= 1;
    }
    &raw[..end]
}

pub fn truncated_marker(prefix: &str, raw: &[u8], kind: LineKind, cfg: &Config) -> String {
    let head = safe_preview_bytes(raw, cfg.head_chars);
    let tail = if raw.len() > cfg.head_chars && cfg.tail_chars > 0 {
        safe_preview_bytes(&raw[raw.len().saturating_sub(cfg.tail_chars)..], cfg.tail_chars)
    } else {
        String::new()
    };
    format!(
        "[{} len={} kind={} head='{}' tail='{}']",
        prefix,
        raw.len(),
        kind.as_str(),
        head,
        tail
    )
}

pub fn render_maybe_gated_line(prefix: &str, raw: &[u8], cfg: &Config) -> String {
    let (gate, kind) = should_gate_line(raw, cfg);
    if gate {
        return truncated_marker(prefix, raw, kind, cfg);
    }
    let s = String::from_utf8_lossy(trim_line_endings(raw));
    escape_field(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gates_long_lines() {
        let cfg = Config {
            hard_line_chars: 10,
            soft_line_chars: 5,
            head_chars: 3,
            tail_chars: 3,
            max_fd_rows: 0,
            max_rg_files: 0,
            max_rg_match_lines_per_file: 0,
            sedx_stdin_max_lines: 0,
            sedx_stdin_max_bytes: 0,
        };
        let raw = b"0123456789ABCDEF";
        let (gate, kind) = should_gate_line(raw, &cfg);
        assert!(gate);
        assert_eq!(kind, LineKind::Base64);
    }

    #[test]
    fn does_not_gate_short_plain_lines() {
        let cfg = Config {
            hard_line_chars: 10,
            soft_line_chars: 5,
            head_chars: 3,
            tail_chars: 3,
            max_fd_rows: 0,
            max_rg_files: 0,
            max_rg_match_lines_per_file: 0,
            sedx_stdin_max_lines: 0,
            sedx_stdin_max_bytes: 0,
        };
        let raw = b"abc\n";
        let (gate, kind) = should_gate_line(raw, &cfg);
        assert!(!gate);
        assert_eq!(kind, LineKind::Plain);
    }
}
