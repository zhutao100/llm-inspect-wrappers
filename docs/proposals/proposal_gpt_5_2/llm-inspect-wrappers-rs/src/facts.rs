use anyhow::{Context, Result};
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileFacts {
    pub bytes: u64,
    pub loc_approx: Option<u64>,
    pub max_line_approx: Option<u64>,
    pub is_binary: Option<bool>,
    pub scanned_bytes: u64,
    pub truncated_scan: bool,
}

impl FileFacts {
    pub fn to_kv_fields(&self) -> Vec<String> {
        let mut out = Vec::new();
        out.push(format!("bytes={}", self.bytes));
        if let Some(loc) = self.loc_approx {
            out.push(if self.truncated_scan {
                format!("loc≈{}", loc)
            } else {
                format!("loc={}", loc)
            });
        }
        if let Some(maxl) = self.max_line_approx {
            out.push(if self.truncated_scan {
                format!("max_line≈{}", maxl)
            } else {
                format!("max_line={}", maxl)
            });
        }
        if let Some(bin) = self.is_binary {
            out.push(format!("bin={}", if bin { 1 } else { 0 }));
        }
        out
    }
}

pub struct ScanBudget {
    pub max_bytes: u64,
}

impl Default for ScanBudget {
    fn default() -> Self {
        Self { max_bytes: 4 * 1024 * 1024 }
    }
}

pub fn file_facts(path: &Path, budget: ScanBudget) -> Result<FileFacts> {
    let md = std::fs::metadata(path)
        .with_context(|| format!("metadata failed for {}", path.display()))?;

    let bytes = md.len();
    if !md.is_file() {
        return Ok(FileFacts {
            bytes,
            loc_approx: None,
            max_line_approx: None,
            is_binary: None,
            scanned_bytes: 0,
            truncated_scan: false,
        });
    }

    // Best-effort bounded scan: count newlines and track max line length.
    let mut f = File::open(path).with_context(|| format!("open failed for {}", path.display()))?;

    let mut buf = [0u8; 64 * 1024];
    let mut scanned: u64 = 0;
    let mut loc: u64 = 0;
    let mut max_line: u64 = 0;
    let mut cur_line: u64 = 0;
    let mut saw_nul = false;

    loop {
        if scanned >= budget.max_bytes {
            break;
        }
        let to_read = (budget.max_bytes - scanned).min(buf.len() as u64) as usize;
        let n = f.read(&mut buf[..to_read]).with_context(|| {
            format!("read failed for {} (scanned={})", path.display(), scanned)
        })?;
        if n == 0 {
            break;
        }
        scanned += n as u64;

        for &b in &buf[..n] {
            if b == 0 {
                saw_nul = true;
            }
            if b == b'\n' {
                loc += 1;
                if cur_line > max_line {
                    max_line = cur_line;
                }
                cur_line = 0;
            } else {
                cur_line += 1;
            }
        }
    }

    // If file doesn't end with \n, we still have a last line.
    if cur_line > 0 {
        if cur_line > max_line {
            max_line = cur_line;
        }
        loc += 1;
    }

    Ok(FileFacts {
        bytes,
        loc_approx: Some(loc),
        max_line_approx: Some(max_line),
        is_binary: Some(saw_nul),
        scanned_bytes: scanned,
        truncated_scan: scanned < bytes,
    })
}
