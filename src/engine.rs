use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;
use regex::Regex;

use crate::error::{FQueryError, Result};
use crate::parser::{AggExpr, CmpOp, Column, ColumnExpr, Condition, FileField, OrderByExpr, Query, SelectClause, SelectItem, SelectItemKind, SliceRange, Transform, WhereClause};

/// Shared file-level metadata (one allocation per file, shared across rows).
#[derive(Debug, Clone)]
struct FileData {
    filepath: String,
    filename: String,
    filedir: String,
    fileext: String,
    filesize: u64,
    modified: String,
    created: String,
    linecount: usize,
    wordcount: usize,
    charcount: usize,
    /// Only populated when CONTENT column is used in the query.
    content: Option<String>,
    is_binary: bool,
}

/// A single result row.
#[derive(Debug, Clone)]
pub struct Row {
    file: Arc<FileData>,
    pub lineno: usize,
    pub line: String,
    pub count: Option<usize>,
    /// Computed aggregate values keyed by expression string (e.g. "SUM(LINECOUNT)").
    pub agg_values: std::collections::HashMap<String, String>,
}

// Public accessors for backwards compatibility
impl Row {
    pub fn filepath(&self) -> &str { &self.file.filepath }
    pub fn filename(&self) -> &str { &self.file.filename }
    pub fn filedir(&self) -> &str { &self.file.filedir }
    pub fn fileext(&self) -> &str { &self.file.fileext }
    pub fn filesize(&self) -> u64 { self.file.filesize }
    pub fn modified(&self) -> &str { &self.file.modified }
    pub fn created(&self) -> &str { &self.file.created }
    pub fn linecount(&self) -> usize { self.file.linecount }
    pub fn wordcount(&self) -> usize { self.file.wordcount }
    pub fn charcount(&self) -> usize { self.file.charcount }
    pub fn content(&self) -> &str { self.file.content.as_deref().unwrap_or("") }
    pub fn filetype(&self) -> &str { if self.file.is_binary { "binary" } else { "text" } }
}

impl Row {
    fn get_column(&self, col: &Column) -> String {
        match col {
            Column::Line => self.line.clone(),
            Column::LineNo => self.lineno.to_string(),
            Column::FileName => self.filename().to_string(),
            Column::FilePath => self.filepath().to_string(),
            Column::FileDir => self.filedir().to_string(),
            Column::FileExt => self.fileext().to_string(),
            Column::FileSize => self.filesize().to_string(),
            Column::Modified => self.modified().to_string(),
            Column::Created => self.created().to_string(),
            Column::LineCount => self.linecount().to_string(),
            Column::WordCount => self.wordcount().to_string(),
            Column::CharCount => self.charcount().to_string(),
            Column::Content => self.content().to_string(),
            Column::FileType => self.filetype().to_string(),
        }
    }

    fn get_column_expr(&self, expr: &ColumnExpr) -> String {
        let mut val = self.get_column(&expr.col);

        // Apply transforms in order
        for t in &expr.transforms {
            val = match t {
                Transform::Trim(chars) => trim_with_chars(&val, chars, TrimMode::Both),
                Transform::TrimStart(chars) => trim_with_chars(&val, chars, TrimMode::Start),
                Transform::TrimEnd(chars) => trim_with_chars(&val, chars, TrimMode::End),
                Transform::Upper => val.to_uppercase(),
                Transform::Lower => val.to_lowercase(),
                Transform::Split(delim, idx) => {
                    val.split(delim.as_str())
                        .nth(*idx)
                        .unwrap_or("")
                        .to_string()
                }
                Transform::Replace(from, to) => val.replace(from.as_str(), to.as_str()),
            };
        }

        // Apply slice
        match &expr.slice {
            None => val,
            Some(slice) => {
                let chars: Vec<char> = val.chars().collect();
                let len = chars.len();
                match slice {
                    SliceRange::Index(i) => {
                        if *i < len { chars[*i].to_string() } else { String::new() }
                    }
                    SliceRange::RangeTo(end) => {
                        let end = (*end).min(len);
                        chars[..end].iter().collect()
                    }
                    SliceRange::RangeFrom(start) => {
                        if *start < len { chars[*start..].iter().collect() } else { String::new() }
                    }
                    SliceRange::Range(start, end) => {
                        let start = (*start).min(len);
                        let end = (*end).min(len);
                        if start < end { chars[start..end].iter().collect() } else { String::new() }
                    }
                }
            }
        }
    }

    fn resolve_var(&self, name: &str) -> Option<String> {
        match name {
            "LINE" => Some(self.line.clone()),
            "LINENO" => Some(self.lineno.to_string()),
            "FILENAME" => Some(self.filename().to_string()),
            "FILEPATH" => Some(self.filepath().to_string()),
            "FILEDIR" => Some(self.filedir().to_string()),
            "FILEEXT" => Some(self.fileext().to_string()),
            "FILESIZE" => Some(self.filesize().to_string()),
            "MODIFIED" => Some(self.modified().to_string()),
            "CREATED" => Some(self.created().to_string()),
            "COUNT(*)" => Some(self.count.unwrap_or(0).to_string()),
            "LINECOUNT" => Some(self.linecount().to_string()),
            "WORDCOUNT" => Some(self.wordcount().to_string()),
            "CHARCOUNT" => Some(self.charcount().to_string()),
            "CONTENT" => Some(self.content().to_string()),
            "FILETYPE" => Some(self.filetype().to_string()),
            _ => None,
        }
    }

    fn resolve_var_numeric(&self, name: &str) -> Option<f64> {
        match name {
            "FILESIZE" => Some(self.filesize() as f64),
            "LINENO" => Some(self.lineno as f64),
            "COUNT(*)" => Some(self.count.unwrap_or(0) as f64),
            "LINECOUNT" => Some(self.linecount() as f64),
            "WORDCOUNT" => Some(self.wordcount() as f64),
            "CHARCOUNT" => Some(self.charcount() as f64),
            _ => None,
        }
    }

    /// Evaluate an expression inside {}: variable optionally followed by arithmetic.
    /// Supports: {VAR}, {VAR+N}, {VAR-N}, {VAR*N}, {VAR/N}, {VAR%N}
    /// Chainable: {FILESIZE/1024/1024}
    fn eval_expr(&self, expr: &str) -> String {
        let expr = expr.trim();

        // Check aggregate values first (e.g. SUM(LINECOUNT), COUNT(*), AVG(FILESIZE))
        // Find the longest matching agg key
        for (key, val) in &self.agg_values {
            if expr.starts_with(key.as_str()) {
                let rest = &expr[key.len()..];
                if rest.is_empty() {
                    return val.clone();
                }
                // Has arithmetic after aggregate — parse as f64
                if let Ok(num) = val.parse::<f64>() {
                    if let Some(result) = Self::apply_ops(num, rest) {
                        return format_f64(result);
                    }
                }
                return val.clone();
            }
        }

        // Handle function-style variables like COUNT(*)
        let (var_name, rest) = if expr.starts_with("COUNT(*)") {
            ("COUNT(*)", &expr[8..])
        } else {
            let var_end = expr
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(expr.len());
            (&expr[..var_end], &expr[var_end..])
        };

        // No arithmetic — just resolve the variable
        if rest.is_empty() {
            return self.resolve_var(var_name).unwrap_or_else(|| format!("{{{expr}}}"));
        }

        // Has arithmetic — need numeric value
        let base = match self.resolve_var_numeric(var_name) {
            Some(v) => v,
            None => return format!("{{{expr}}}"), // not a numeric var
        };

        let result = Self::apply_ops(base, rest);
        match result {
            Some(v) if v == v.trunc() => format!("{}", v as i64),
            Some(v) => format!("{:.2}", v),
            None => format!("{{{expr}}}"),
        }
    }

    fn apply_ops(mut val: f64, ops: &str) -> Option<f64> {
        let mut chars = ops.chars().peekable();
        while let Some(&op) = chars.peek() {
            if !matches!(op, '+' | '-' | '*' | '/' | '%') {
                return None;
            }
            chars.next();
            // Parse number (supports decimals)
            let mut num_str = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() || c == '.' {
                    num_str.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            let num: f64 = match num_str.parse() {
                Ok(n) => n,
                Err(_) => return None,
            };
            val = match op {
                '+' => val + num,
                '-' => val - num,
                '*' => val * num,
                '/' => if num != 0.0 { val / num } else { return None },
                '%' => if num != 0.0 { val % num } else { return None },
                _ => return None,
            };
        }
        Some(val)
    }

    fn format_template(&self, template: &str) -> String {
        let mut out = String::with_capacity(template.len());
        let chars: Vec<char> = template.chars().collect();
        let len = chars.len();
        let mut i = 0;
        while i < len {
            if chars[i] == '\\' && i + 1 < len && (chars[i + 1] == '{' || chars[i + 1] == '}') {
                out.push(chars[i + 1]);
                i += 2;
            } else if chars[i] == '{' {
                if let Some(end) = chars[i..].iter().position(|&c| c == '}') {
                    let expr: String = chars[i + 1..i + end].iter().collect();
                    out.push_str(&self.eval_expr(&expr));
                    i += end + 1;
                } else {
                    out.push(chars[i]);
                    i += 1;
                }
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        out
    }
}

/// Aggregated query result.
#[derive(Debug)]
pub enum QueryResult {
    Rows(Vec<Row>),
    Numeric(f64),
}

/// Recursively collect all files under a directory.
fn collect_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|e| FQueryError::Io {
        path: dir.to_path_buf(),
        source: e,
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| FQueryError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        let ft = entry.file_type().map_err(|e| FQueryError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        if ft.is_dir() {
            collect_files_recursive(&entry.path(), files)?;
        } else if ft.is_file() {
            files.push(entry.path());
        }
    }
    Ok(())
}

/// Resolve the FROM clause into a list of file paths.
fn resolve_files(pattern: &str) -> Result<Vec<PathBuf>> {
    if pattern == "*" {
        let entries: Vec<PathBuf> = glob::glob("*")
            .map_err(FQueryError::GlobPattern)?
            .filter_map(|e| e.ok())
            .filter(|p| p.is_file())
            .collect();
        return Ok(entries);
    }

    // If it contains glob chars, treat as glob
    if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
        let entries: Vec<PathBuf> = glob::glob(pattern)
            .map_err(FQueryError::GlobPattern)?
            .filter_map(|e| e.ok())
            .filter(|p| p.is_file())
            .collect();
        return Ok(entries);
    }

    // Otherwise treat as literal path
    let path = PathBuf::from(pattern);
    if path.is_file() {
        Ok(vec![path])
    } else if path.is_dir() {
        // Scan directory recursively
        let mut files = Vec::new();
        collect_files_recursive(&path, &mut files)?;
        Ok(files)
    } else {
        Ok(vec![])
    }
}

/// Pre-computed file info passed into condition evaluation.
struct FileInfo {
    /// Full path as string
    path: String,
    /// Just the file name (e.g. "main.rs")
    name: String,
    /// Directory part (e.g. "/home/user/src")
    dir: String,
    /// Extension only (e.g. "rs"), empty if none
    ext: String,
    /// File size in bytes
    size: u64,
    /// Last modified time as "YYYY-MM-DD HH:MM:SS"
    modified: String,
    /// Creation time as "YYYY-MM-DD HH:MM:SS" (may equal modified on some OS)
    created: String,
    is_binary: bool,
}

impl FileInfo {
    /// Create FileInfo from metadata only (no file open). is_binary defaults to false.
    fn from_path(path: &Path) -> Result<Self> {
        let meta = std::fs::metadata(path).map_err(|e| FQueryError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        let path_str = path.to_string_lossy().to_string();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let dir = path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();

        let modified = meta
            .modified()
            .ok()
            .map(format_system_time)
            .unwrap_or_default();
        let created = meta
            .created()
            .ok()
            .map(format_system_time)
            .unwrap_or_default();

        Ok(Self {
            path: path_str,
            name,
            dir,
            ext,
            size: meta.len(),
            modified,
            created,
            is_binary: false, // determined lazily after pre-filtering
        })
    }

    /// Check if file is binary by reading first 512 bytes.
    fn check_binary(path: &Path) -> bool {
        use std::io::Read;
        let mut buf = [0u8; 512];
        std::fs::File::open(path)
            .and_then(|mut f| f.read(&mut buf))
            .map(|n| buf[..n].contains(&0))
            .unwrap_or(false)
    }

    fn get_field(&self, field: &FileField) -> &str {
        match field {
            FileField::Name => &self.name,
            FileField::Path => &self.path,
            FileField::Dir => &self.dir,
            FileField::Ext => &self.ext,
            FileField::Type => if self.is_binary { "binary" } else { "text" },
        }
    }
}

fn format_system_time(t: std::time::SystemTime) -> String {
    let duration = t
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Simple UTC formatting without external crate
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since epoch to Y-M-D (simplified algorithm)
    let (year, month, day) = epoch_days_to_date(days);
    format!("{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02}:{seconds:02}")
}

fn epoch_days_to_date(days: u64) -> (u64, u64, u64) {
    // Civil calendar from days since 1970-01-01
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn cmp_op_eval_f64(op: &CmpOp, a: f64, b: f64) -> bool {
    match op {
        CmpOp::Eq => a == b,
        CmpOp::Neq => a != b,
        CmpOp::Gt => a > b,
        CmpOp::Lt => a < b,
        CmpOp::Gte => a >= b,
        CmpOp::Lte => a <= b,
    }
}

fn cmp_op_eval<T: PartialOrd>(op: &CmpOp, a: &T, b: &T) -> bool {
    match op {
        CmpOp::Eq => a == b,
        CmpOp::Neq => a != b,
        CmpOp::Gt => a > b,
        CmpOp::Lt => a < b,
        CmpOp::Gte => a >= b,
        CmpOp::Lte => a <= b,
    }
}

/// Convert SQL LIKE pattern to regex: % → .*, _ → ., rest escaped.
fn like_to_regex(pattern: &str) -> String {
    let mut re = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '%' => re.push_str(".*"),
            '_' => re.push('.'),
            c => {
                if "\\.|+*?()[]{}^$#&-~".contains(c) {
                    re.push('\\');
                }
                re.push(c);
            }
        }
    }
    re.push('$');
    re
}

enum TrimMode { Both, Start, End }

fn trim_with_chars(val: &str, chars: &Option<String>, mode: TrimMode) -> String {
    match chars {
        None => match mode {
            TrimMode::Both => val.trim().to_string(),
            TrimMode::Start => val.trim_start().to_string(),
            TrimMode::End => val.trim_end().to_string(),
        },
        Some(c) => {
            let pat: &[char] = &c.chars().collect::<Vec<_>>();
            match mode {
                TrimMode::Both => val.trim_matches(pat).to_string(),
                TrimMode::Start => val.trim_start_matches(pat).to_string(),
                TrimMode::End => val.trim_end_matches(pat).to_string(),
            }
        }
    }
}

/// Resolve a ColumnExpr value in WHERE context (no Row available).
fn resolve_column_in_where(col: &ColumnExpr, line: &str, lineno: usize, file_info: &FileInfo) -> String {
    let raw = match col.col {
        Column::Line => line.to_string(),
        Column::LineNo => lineno.to_string(),
        Column::FileName => file_info.name.clone(),
        Column::FilePath => file_info.path.clone(),
        Column::FileDir => file_info.dir.clone(),
        Column::FileExt => file_info.ext.clone(),
        Column::FileType => if file_info.is_binary { "binary".into() } else { "text".into() },
        Column::FileSize => file_info.size.to_string(),
        Column::Modified => file_info.modified.clone(),
        Column::Created => file_info.created.clone(),
        _ => String::new(),
    };
    let mut val = raw;
    for t in &col.transforms {
        val = match t {
            Transform::Trim(chars) => trim_with_chars(&val, chars, TrimMode::Both),
            Transform::TrimStart(chars) => trim_with_chars(&val, chars, TrimMode::Start),
            Transform::TrimEnd(chars) => trim_with_chars(&val, chars, TrimMode::End),
            Transform::Upper => val.to_uppercase(),
            Transform::Lower => val.to_lowercase(),
            Transform::Split(delim, idx) => {
                val.split(delim.as_str()).nth(*idx).unwrap_or("").to_string()
            }
            Transform::Replace(from, to) => val.replace(from.as_str(), to.as_str()),
        };
    }
    if let Some(slice) = &col.slice {
        let chars: Vec<char> = val.chars().collect();
        let len = chars.len();
        val = match slice {
            SliceRange::Index(i) => if *i < len { chars[*i].to_string() } else { String::new() },
            SliceRange::RangeTo(end) => chars[..(*end).min(len)].iter().collect(),
            SliceRange::RangeFrom(start) => if *start < len { chars[*start..].iter().collect() } else { String::new() },
            SliceRange::Range(start, end) => {
                let s = (*start).min(len);
                let e = (*end).min(len);
                if s < e { chars[s..e].iter().collect() } else { String::new() }
            }
        };
    }
    val
}

fn matches_condition(
    cond: &Condition,
    line: &str,
    lineno: usize,
    file_info: &FileInfo,
    compiled_regexes: &CompiledRegexes,
) -> Result<bool> {
    match cond {
        Condition::LineEquals(s) => Ok(line == s.as_str()),
        Condition::LineNotEquals(s) => Ok(line != s.as_str()),
        Condition::LineContains(s) => Ok(line.contains(s.as_str())),
        Condition::LineNotContains(s) => Ok(!line.contains(s.as_str())),
        Condition::LineMatches(pat) => {
            let re = compiled_regexes
                .get(pat)
                .ok_or_else(|| FQueryError::Parse(format!("regex not compiled: {pat}")))?;
            Ok(re.is_match(line))
        }
        Condition::LineNotMatches(pat) => {
            let re = compiled_regexes
                .get(pat)
                .ok_or_else(|| FQueryError::Parse(format!("regex not compiled: {pat}")))?;
            Ok(!re.is_match(line))
        }
        Condition::LineNoEquals(n) => Ok(lineno == *n),
        Condition::LineNoGreaterThan(n) => Ok(lineno > *n),
        Condition::LineNoLessThan(n) => Ok(lineno < *n),
        Condition::LineNoGreaterOrEqual(n) => Ok(lineno >= *n),
        Condition::LineNoLessOrEqual(n) => Ok(lineno <= *n),
        Condition::FileFieldEquals(field, val, negated) => {
            let v = file_info.get_field(field) == val.as_str();
            Ok(if *negated { !v } else { v })
        }
        Condition::FileFieldContains(field, val, negated) => {
            let v = file_info.get_field(field).contains(val.as_str());
            Ok(if *negated { !v } else { v })
        }
        Condition::FileFieldMatches(field, pat, negated) => {
            let re = compiled_regexes
                .get(pat)
                .ok_or_else(|| FQueryError::Parse(format!("regex not compiled: {pat}")))?;
            let v = re.is_match(file_info.get_field(field));
            Ok(if *negated { !v } else { v })
        }
        Condition::LineLike(pat, negated) => {
            let re_pat = like_to_regex(pat);
            let re = compiled_regexes
                .get(&re_pat)
                .ok_or_else(|| FQueryError::Parse(format!("like regex not compiled: {pat}")))?;
            let v = re.is_match(line);
            Ok(if *negated { !v } else { v })
        }
        Condition::FileFieldLike(field, pat, negated) => {
            let re_pat = like_to_regex(pat);
            let re = compiled_regexes
                .get(&re_pat)
                .ok_or_else(|| FQueryError::Parse(format!("like regex not compiled: {pat}")))?;
            let v = re.is_match(file_info.get_field(field));
            Ok(if *negated { !v } else { v })
        }
        Condition::LenCmp(col, op, val) => {
            let val_str = resolve_column_in_where(col, line, lineno, file_info);
            let len = val_str.chars().count();
            Ok(cmp_op_eval(op, &len, val))
        }
        Condition::ExprEquals(expr, val, negated) => {
            let v = resolve_column_in_where(expr, line, lineno, file_info) == *val;
            Ok(if *negated { !v } else { v })
        }
        Condition::ExprContains(expr, val, negated) => {
            let v = resolve_column_in_where(expr, line, lineno, file_info).contains(val.as_str());
            Ok(if *negated { !v } else { v })
        }
        Condition::ExprMatches(expr, pat, negated) => {
            let re = compiled_regexes.get(pat)
                .ok_or_else(|| FQueryError::Parse(format!("regex not compiled: {pat}")))?;
            let v = re.is_match(&resolve_column_in_where(expr, line, lineno, file_info));
            Ok(if *negated { !v } else { v })
        }
        Condition::ExprLike(expr, pat, negated) => {
            let re_pat = like_to_regex(pat);
            let re = compiled_regexes.get(&re_pat)
                .ok_or_else(|| FQueryError::Parse(format!("like regex not compiled: {pat}")))?;
            let v = re.is_match(&resolve_column_in_where(expr, line, lineno, file_info));
            Ok(if *negated { !v } else { v })
        }
        Condition::FileSizeBetween(low, high) => {
            Ok(file_info.size >= *low && file_info.size <= *high)
        }
        Condition::FileFieldIn(field, values, negated) => {
            let v = values.iter().any(|val| file_info.get_field(field) == val.as_str());
            Ok(if *negated { !v } else { v })
        }
        Condition::FileSizeCmp(op, val) => Ok(cmp_op_eval(op, &file_info.size, val)),
        Condition::ModifiedCmp(op, val) => Ok(cmp_op_eval(op, &file_info.modified, val)),
        Condition::CreatedCmp(op, val) => Ok(cmp_op_eval(op, &file_info.created, val)),
    }
}

fn eval_where(
    clause: &WhereClause,
    line: &str,
    lineno: usize,
    file_info: &FileInfo,
    regexes: &CompiledRegexes,
) -> Result<bool> {
    match clause {
        WhereClause::Single(c) => matches_condition(c, line, lineno, file_info, regexes),
        WhereClause::And(parts) => {
            for p in parts {
                if !eval_where(p, line, lineno, file_info, regexes)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        WhereClause::Or(parts) => {
            for p in parts {
                if eval_where(p, line, lineno, file_info, regexes)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        WhereClause::Not(inner) => {
            let val = eval_where(inner, line, lineno, file_info, regexes)?;
            Ok(!val)
        }
    }
}

/// Pre-compiled regexes keyed by their pattern string.
struct CompiledRegexes {
    map: std::collections::HashMap<String, Regex>,
}

impl CompiledRegexes {
    fn get(&self, pattern: &str) -> Option<&Regex> {
        self.map.get(pattern)
    }
}

/// Collect all regex patterns from conditions so we can compile them once.
fn collect_patterns(clause: &WhereClause, patterns: &mut Vec<String>) {
    match clause {
        WhereClause::Single(c) => match c {
            Condition::LineMatches(p)
            | Condition::LineNotMatches(p) => {
                if !patterns.contains(p) {
                    patterns.push(p.clone());
                }
            }
            Condition::FileFieldMatches(_, p, _) => {
                if !patterns.contains(p) {
                    patterns.push(p.clone());
                }
            }
            Condition::LineLike(p, _) | Condition::FileFieldLike(_, p, _) | Condition::ExprLike(_, p, _) => {
                let re_pat = like_to_regex(p);
                if !patterns.contains(&re_pat) {
                    patterns.push(re_pat);
                }
            }
            Condition::ExprMatches(_, p, _) => {
                if !patterns.contains(p) {
                    patterns.push(p.clone());
                }
            }
            _ => {}
        },
        WhereClause::And(parts) | WhereClause::Or(parts) => {
            for p in parts {
                collect_patterns(p, patterns);
            }
        }
        WhereClause::Not(inner) => collect_patterns(inner, patterns),
    }
}

fn compile_regexes(query: &Query) -> Result<CompiledRegexes> {
    let mut patterns = Vec::new();
    if let Some(ref w) = query.where_clause {
        collect_patterns(w, &mut patterns);
    }
    let mut map = std::collections::HashMap::new();
    for pat in patterns {
        let re = Regex::new(&pat)?;
        map.insert(pat, re);
    }
    Ok(CompiledRegexes { map })
}

fn is_file_condition(c: &Condition) -> bool {
    match c {
        Condition::FileFieldEquals(..)
        | Condition::FileFieldContains(..)
        | Condition::FileFieldMatches(..)
        | Condition::FileFieldLike(..)
        | Condition::FileSizeCmp(..)
        | Condition::FileSizeBetween(..)
        | Condition::FileFieldIn(..)
        | Condition::ModifiedCmp(..)
        | Condition::CreatedCmp(..) => true,
        Condition::LenCmp(col, _, _) => !is_line_level_column(&col.col),
        Condition::ExprEquals(col, _, _)
        | Condition::ExprContains(col, _, _)
        | Condition::ExprMatches(col, _, _)
        | Condition::ExprLike(col, _, _) => !is_line_level_column(&col.col),
        _ => false,
    }
}

/// Check if a file can be excluded based on file-level conditions before reading content.
fn file_excluded(where_clause: &Option<WhereClause>, file_info: &FileInfo, regexes: &CompiledRegexes) -> Result<bool> {
    match where_clause {
        Some(clause) => {
            let result = eval_file_filter(clause, file_info, regexes)?;
            Ok(result == Some(false))
        }
        None => Ok(false),
    }
}

/// Try to evaluate a where clause using only file-level information.
/// Returns Some(bool) if deterministic, None if line data is needed.
fn eval_file_filter(clause: &WhereClause, file_info: &FileInfo, regexes: &CompiledRegexes) -> Result<Option<bool>> {
    match clause {
        WhereClause::Single(c) if is_file_condition(c) => {
            // Evaluate with dummy line data — file conditions don't use it
            let result = matches_condition(c, "", 0, file_info, regexes)?;
            Ok(Some(result))
        }
        WhereClause::Single(_) => Ok(None),
        WhereClause::And(parts) => {
            for p in parts {
                if let Some(false) = eval_file_filter(p, file_info, regexes)? {
                    return Ok(Some(false));
                }
            }
            Ok(None)
        }
        WhereClause::Or(parts) => {
            for p in parts {
                if let Some(true) = eval_file_filter(p, file_info, regexes)? {
                    return Ok(Some(true));
                }
            }
            Ok(None)
        }
        WhereClause::Not(inner) => {
            match eval_file_filter(inner, file_info, regexes)? {
                Some(v) => Ok(Some(!v)),
                None => Ok(None),
            }
        }
    }
}

fn process_content(
    content: &str,
    file_info: &FileInfo,
    where_clause: &Option<WhereClause>,
    regexes: &CompiledRegexes,
    limit: Option<&AtomicUsize>,
    needs_content: bool,
) -> Result<Vec<Row>> {
    let file_data = Arc::new(FileData {
        filepath: file_info.path.clone(),
        filename: file_info.name.clone(),
        filedir: file_info.dir.clone(),
        fileext: file_info.ext.clone(),
        filesize: file_info.size,
        modified: file_info.modified.clone(),
        created: file_info.created.clone(),
        linecount: content.lines().count(),
        wordcount: content.split_whitespace().count(),
        charcount: content.chars().count(),
        content: if needs_content { Some(content.to_string()) } else { None },
        is_binary: false,
    });
    let mut rows = Vec::new();

    for (idx, line) in content.lines().enumerate() {
        // Check if global limit reached
        if let Some(counter) = limit {
            if counter.load(Ordering::Relaxed) == 0 {
                break;
            }
        }

        let lineno = idx + 1;
        let matches = match where_clause {
            Some(w) => eval_where(w, line, lineno, &file_info, regexes)?,
            None => true,
        };

        if matches {
            if let Some(counter) = limit {
                // Try to claim a slot
                let prev = counter.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                    if v > 0 { Some(v - 1) } else { None }
                });
                if prev.is_err() {
                    break;
                }
            }

            rows.push(Row {
                file: Arc::clone(&file_data),
                lineno,
                line: line.to_string(),
                count: None,
                agg_values: std::collections::HashMap::new(),
            });
        }
    }

    Ok(rows)
}

/// Check if query needs LINECOUNT, WORDCOUNT, or CHARCOUNT.
fn query_needs_counts(query: &Query) -> bool {
    let check_col = |col: &Column| matches!(col, Column::LineCount | Column::WordCount | Column::CharCount);
    let check_expr = |expr: &ColumnExpr| check_col(&expr.col);
    let check_agg = |agg: &AggExpr| match agg {
        AggExpr::CountAll => false,
        AggExpr::CountCol(c) | AggExpr::Min(c) | AggExpr::Max(c)
        | AggExpr::Sum(c) | AggExpr::Avg(c) => check_expr(c),
    };
    let SelectClause::Items(items) = &query.select;
    items.iter().any(|item| match item {
        SelectItem { kind: SelectItemKind::Column(c), .. } => check_expr(c),
        SelectItem { kind: SelectItemKind::Format(t), .. } => t.contains("LINECOUNT") || t.contains("WORDCOUNT") || t.contains("CHARCOUNT"),
        SelectItem { kind: SelectItemKind::Aggregate(a), .. } => check_agg(a),
    })
}

/// Does a column require reading the file?
fn column_needs_file_open(col: &Column) -> bool {
    matches!(col, Column::Content | Column::LineCount | Column::WordCount | Column::CharCount | Column::FileType | Column::Line | Column::LineNo)
}

fn select_item_needs_file_open(item: &SelectItem) -> bool {
    match &item.kind {
        SelectItemKind::Column(c) => column_needs_file_open(&c.col),
        SelectItemKind::Format(t) => t.contains("{CONTENT") || t.contains("{LINECOUNT")
            || t.contains("{WORDCOUNT") || t.contains("{CHARCOUNT")
            || t.contains("{FILETYPE") || t.contains("{LINE}") || t.contains("{LINENO}"),
        SelectItemKind::Aggregate(agg) => match agg {
            AggExpr::CountAll => false, // COUNT(*) just counts rows, no file read needed
            AggExpr::CountCol(c) | AggExpr::Min(c) | AggExpr::Max(c)
            | AggExpr::Sum(c) | AggExpr::Avg(c) => column_needs_file_open(&c.col),
        },
    }
}

fn where_needs_file_open(clause: &WhereClause) -> bool {
    match clause {
        WhereClause::Single(c) => match c {
            Condition::LineEquals(_) | Condition::LineNotEquals(_)
            | Condition::LineContains(_) | Condition::LineNotContains(_)
            | Condition::LineMatches(_) | Condition::LineNotMatches(_)
            | Condition::LineLike(_, _)
            | Condition::LineNoEquals(_) | Condition::LineNoGreaterThan(_)
            | Condition::LineNoLessThan(_) | Condition::LineNoGreaterOrEqual(_)
            | Condition::LineNoLessOrEqual(_) => true,
            Condition::FileFieldEquals(FileField::Type, _, _)
            | Condition::FileFieldContains(FileField::Type, _, _)
            | Condition::FileFieldMatches(FileField::Type, _, _)
            | Condition::FileFieldLike(FileField::Type, _, _)
            | Condition::FileFieldIn(FileField::Type, _, _) => true,
            Condition::LenCmp(col, _, _) => column_needs_file_open(&col.col),
            _ => false,
        },
        WhereClause::And(parts) | WhereClause::Or(parts) => parts.iter().any(where_needs_file_open),
        WhereClause::Not(inner) => where_needs_file_open(inner),
    }
}

/// Check if query needs to open files at all.
fn query_needs_file_open(query: &Query) -> bool {
    let SelectClause::Items(items) = &query.select;
    if items.iter().any(select_item_needs_file_open) {
        return true;
    }
    if let Some(ref w) = query.where_clause {
        if where_needs_file_open(w) {
            return true;
        }
    }
    if let Some(ref g) = query.group_by {
        if column_needs_file_open(&g.col) {
            return true;
        }
    }
    false
}



fn process_file(
    path: &Path,
    where_clause: &Option<WhereClause>,
    regexes: &CompiledRegexes,
    limit: Option<&AtomicUsize>,
    needs_content: bool,
    file_level_only: bool,
    needs_counts: bool,
    needs_file_open: bool,
) -> Result<Vec<Row>> {
    // Phase 1: stat only — no file open. Skip if file vanished (race condition).
    let mut file_info = match FileInfo::from_path(path) {
        Ok(fi) => fi,
        Err(FQueryError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(e) => return Err(e),
    };

    // Pre-filter by file metadata that doesn't need file open (FILESIZE, FILEEXT, etc.)
    // Exclude FILETYPE from this check since is_binary isn't known yet
    if file_excluded(where_clause, &file_info, regexes)? {
        return Ok(Vec::new());
    }

    // Phase 2: binary check — only if the query needs it (FILETYPE, CONTENT, line-level data)
    if needs_file_open {
        file_info.is_binary = FileInfo::check_binary(path);

        // Re-check with FILETYPE now available
        if file_excluded(where_clause, &file_info, regexes)? {
            return Ok(Vec::new());
        }
    }

    // Binary files: metadata-only Row, no content reading
    if needs_file_open && file_info.is_binary {
        let file_data = Arc::new(FileData {
            filepath: file_info.path.clone(),
            filename: file_info.name.clone(),
            filedir: file_info.dir.clone(),
            fileext: file_info.ext.clone(),
            filesize: file_info.size,
            modified: file_info.modified.clone(),
            created: file_info.created.clone(),
            linecount: 0,
            wordcount: 0,
            charcount: 0,
            content: None,
            is_binary: true,
        });
        return Ok(vec![Row {
            file: file_data,
            lineno: 0,
            line: String::new(),
            count: None,
            agg_values: std::collections::HashMap::new(),
        }]);
    }

    // File-level only: one Row per file
    if file_level_only {
        // If we don't need counts, skip reading entirely — just use metadata
        let (linecount, wordcount, charcount) = if needs_counts {
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData => return Ok(Vec::new()),
                Err(e) => return Err(FQueryError::Io { path: path.to_path_buf(), source: e }),
            };
            let lc = content.lines().count();
            let wc = content.split_whitespace().count();
            let cc = content.chars().count();
            // content is dropped here
            (lc, wc, cc)
        } else {
            (0, 0, 0)
        };

        let file_data = Arc::new(FileData {
            filepath: file_info.path.clone(),
            filename: file_info.name.clone(),
            filedir: file_info.dir.clone(),
            fileext: file_info.ext.clone(),
            filesize: file_info.size,
            modified: file_info.modified.clone(),
            created: file_info.created.clone(),
            linecount,
            wordcount,
            charcount,
            content: None,
            is_binary: false,
        });
        return Ok(vec![Row {
            file: file_data,
            lineno: 0,
            line: String::new(),
            count: None,
            agg_values: std::collections::HashMap::new(),
        }]);
    }

    // Full line-level processing
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => return Ok(Vec::new()),
        Err(e) => return Err(FQueryError::Io { path: path.to_path_buf(), source: e }),
    };

    process_content(&content, &file_info, where_clause, regexes, limit, needs_content)
}

fn process_stdin(
    where_clause: &Option<WhereClause>,
    regexes: &CompiledRegexes,
    limit: Option<&AtomicUsize>,
    needs_content: bool,
) -> Result<Vec<Row>> {
    use std::io::Read;
    let mut content = String::new();
    std::io::stdin().read_to_string(&mut content).map_err(|e| FQueryError::Io {
        path: PathBuf::from("<stdin>"),
        source: e,
    })?;

    let file_info = FileInfo {
        path: "<stdin>".into(),
        name: "<stdin>".into(),
        dir: String::new(),
        ext: String::new(),
        size: content.len() as u64,
        modified: String::new(),
        created: String::new(),
        is_binary: false,
    };

    process_content(&content, &file_info, where_clause, regexes, limit, needs_content)
}

/// Check if a column is line-level (varies per line within a file).
fn is_line_level_column(col: &Column) -> bool {
    matches!(col, Column::Line | Column::LineNo)
}

fn select_item_needs_lines(item: &SelectItem) -> bool {
    match item {
        SelectItem { kind: SelectItemKind::Column(c), .. } => is_line_level_column(&c.col),
        SelectItem { kind: SelectItemKind::Format(t), .. } => t.contains("{LINE}") || t.contains("{LINENO}") || t.contains("{COUNT(*)"),
        SelectItem { kind: SelectItemKind::Aggregate(agg), .. } => match agg {
            AggExpr::CountAll => true,
            AggExpr::CountCol(c) | AggExpr::Min(c) | AggExpr::Max(c)
            | AggExpr::Sum(c) | AggExpr::Avg(c) => is_line_level_column(&c.col),
        },
    }
}

/// Check if any WHERE condition references line-level data.
fn where_needs_lines(clause: &WhereClause) -> bool {
    match clause {
        WhereClause::Single(c) => matches!(c,
            Condition::LineEquals(_) | Condition::LineNotEquals(_)
            | Condition::LineContains(_) | Condition::LineNotContains(_)
            | Condition::LineMatches(_) | Condition::LineNotMatches(_)
            | Condition::LineLike(_, _)
            | Condition::LineNoEquals(_) | Condition::LineNoGreaterThan(_)
            | Condition::LineNoLessThan(_) | Condition::LineNoGreaterOrEqual(_)
            | Condition::LineNoLessOrEqual(_)
        ),
        WhereClause::And(parts) | WhereClause::Or(parts) => parts.iter().any(where_needs_lines),
        WhereClause::Not(inner) => where_needs_lines(inner),
    }
}

/// Check if the query can operate at file-level only (one Row per file, no line scanning).
fn query_is_file_level_only(query: &Query) -> bool {
    let SelectClause::Items(items) = &query.select;
    if items.iter().any(select_item_needs_lines) {
        return false;
    }
    if let Some(ref w) = query.where_clause {
        if where_needs_lines(w) {
            return false;
        }
    }
    if let Some(ref g) = query.group_by {
        if is_line_level_column(&g.col) {
            return false;
        }
    }
    true
}

/// Check if query references CONTENT column.
fn query_needs_content(query: &Query) -> bool {
    let SelectClause::Items(items) = &query.select;
    items.iter().any(|item| match item {
        SelectItem { kind: SelectItemKind::Column(c), .. } => matches!(c.col, Column::Content),
        SelectItem { kind: SelectItemKind::Format(t), .. } => t.contains("{CONTENT"),
        SelectItem { kind: SelectItemKind::Aggregate(agg), .. } => match agg {
            AggExpr::CountAll => false,
            AggExpr::CountCol(c) | AggExpr::Min(c) | AggExpr::Max(c)
            | AggExpr::Sum(c) | AggExpr::Avg(c) => matches!(c.col, Column::Content),
        },
    })
}

/// Execute a parsed query, returning results.
pub fn execute(query: &Query) -> Result<QueryResult> {
    let regexes = compile_regexes(query)?;
    let needs_content = query_needs_content(query);
    let file_level_only = query_is_file_level_only(query);
    let needs_counts = query_needs_counts(query);
    let needs_file_open = query_needs_file_open(query);

    let limit_counter = match (query.limit, query.offset) {
        (Some(l), None) => Some(AtomicUsize::new(l.count)),
        _ => None,
    };

    let mut all_rows = if query.from == "-" {
        process_stdin(&query.where_clause, &regexes, limit_counter.as_ref(), needs_content)?
    } else {
        let files = resolve_files(&query.from)?;
        let results: Vec<Result<Vec<Row>>> = files
            .par_iter()
            .map(|f| process_file(f, &query.where_clause, &regexes, limit_counter.as_ref(), needs_content, file_level_only, needs_counts, needs_file_open))
            .collect();

        let mut rows = Vec::new();
        for r in results {
            rows.extend(r?);
        }
        rows
    };

    // Check if select contains any aggregates
    let has_aggregates = matches!(&query.select, SelectClause::Items(items) if items.iter().any(|i| matches!(i, SelectItem { kind: SelectItemKind::Aggregate(_), .. })));

    // GROUP BY: collapse rows into groups
    if let Some(ref group_by) = query.group_by {
        let mut groups: std::collections::HashMap<String, Vec<Row>> =
            std::collections::HashMap::new();
        for row in all_rows {
            let key = row.get_column_expr(group_by);
            groups.entry(key).or_default().push(row);
        }
        let mut grouped: Vec<(String, Vec<Row>)> = groups.into_iter().collect();
        grouped.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

        // HAVING: filter groups by aggregate conditions (all must pass)
        if let Some(ref having) = query.having {
            grouped.retain(|(_key, group_rows)| {
                having.conditions.iter().all(|cond| {
                    let val_str = eval_aggregate(&cond.agg, group_rows);
                    let val = val_str.parse::<f64>().unwrap_or(0.0);
                    cmp_op_eval_f64(&cond.op, val, cond.value)
                })
            });
        }

        // Take representative row per group, compute aggregates, set count
        all_rows = grouped
            .into_iter()
            .map(|(_key, group_rows)| {
                let count = group_rows.len();
                let mut row = group_rows[0].clone();
                row.count = Some(count);
                // Store aggregate values for this group
                { let SelectClause::Items(ref items) = query.select;
                    for item in items {
                        if let SelectItem { kind: SelectItemKind::Aggregate(agg), .. } = item {
                            let key = agg_expr_key(agg);
                            let val = eval_aggregate(agg, &group_rows);
                            row.agg_values.insert(key, val);
                        }
                    }
                }
                // Also compute aggregates referenced in format strings
                { let SelectClause::Items(ref items) = query.select;
                    for item in items {
                        if let SelectItem { kind: SelectItemKind::Format(tmpl), .. } = item {
                            for agg in extract_template_aggregates(tmpl) {
                                let key = agg_expr_key(&agg);
                                let val = eval_aggregate(&agg, &group_rows);
                                row.agg_values.insert(key, val);
                            }
                        }
                    }
                }
                row
            })
            .collect();
    } else if has_aggregates {
        // Aggregates without GROUP BY → all rows are one group
        // If it's purely aggregates, return single row
        let mut row = if all_rows.is_empty() {
            // Create a dummy row for empty results
            return Ok(QueryResult::Numeric(0.0));
        } else {
            all_rows[0].clone()
        };
        row.count = Some(all_rows.len());
        { let SelectClause::Items(ref items) = query.select;
            for item in items {
                if let SelectItem { kind: SelectItemKind::Aggregate(agg), .. } = item {
                    let key = agg_expr_key(agg);
                    let val = eval_aggregate(agg, &all_rows);
                    row.agg_values.insert(key, val);
                }
            }
        }
        all_rows = vec![row];
    }

    // ORDER BY
    if let Some(ref order) = query.order_by {
        all_rows.sort_by(|a, b| {
            let (va, vb) = match &order.expr {
                OrderByExpr::Column(col) => (
                    a.get_column_expr(col),
                    b.get_column_expr(col),
                ),
                OrderByExpr::Aggregate(agg) => {
                    let key = agg_expr_key(agg);
                    let va = a.agg_values.get(&key).cloned().unwrap_or_default();
                    let vb = b.agg_values.get(&key).cloned().unwrap_or_default();
                    // Try numeric, fall back to string
                    let cmp = match (va.parse::<f64>(), vb.parse::<f64>()) {
                        (Ok(na), Ok(nb)) => na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal),
                        _ => va.cmp(&vb),
                    };
                    return if order.desc { cmp.reverse() } else { cmp };
                }
            };
            let cmp = match (va.parse::<f64>(), vb.parse::<f64>()) {
                (Ok(na), Ok(nb)) => na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal),
                _ => va.cmp(&vb),
            };
            if order.desc { cmp.reverse() } else { cmp }
        });
    }

    // OFFSET then LIMIT (after ORDER BY)
    if let Some(off) = query.offset {
        if off < all_rows.len() {
            all_rows = all_rows.split_off(off);
        } else {
            all_rows.clear();
        }
    }
    if let Some(lim) = query.limit {
        all_rows.truncate(lim.count);
    }

    Ok(QueryResult::Rows(all_rows))
}

/// File-level columns have the same value for every row in a file.
/// When aggregating, we only count them once per file.
fn is_file_level_column(col: &Column) -> bool {
    matches!(
        col,
        Column::FileName
            | Column::FilePath
            | Column::FileDir
            | Column::FileExt
            | Column::FileSize
            | Column::Modified
            | Column::Created
            | Column::LineCount
            | Column::WordCount
            | Column::CharCount
            | Column::Content
    )
}

/// For file-level columns, deduplicate values before aggregating
/// (otherwise SUM(LINECOUNT) = linecount × linecount per file).
fn collect_agg_values(col: &ColumnExpr, rows: &[Row]) -> Vec<f64> {
    if is_file_level_column(&col.col) {
        // Deduplicate by filepath — one value per file
        let mut seen = std::collections::HashSet::new();
        rows.iter()
            .filter(|r| seen.insert(r.filepath().to_string()))
            .filter_map(|r| r.get_column_expr(col).parse::<f64>().ok())
            .collect()
    } else {
        rows.iter()
            .filter_map(|r| r.get_column_expr(col).parse::<f64>().ok())
            .collect()
    }
}

/// Produce a string key for an aggregate expression (used in agg_values HashMap and template lookup).
fn agg_expr_key(agg: &AggExpr) -> String {
    match agg {
        AggExpr::CountAll => "COUNT(*)".to_string(),
        AggExpr::CountCol(c) => format!("COUNT({})", column_expr_name(c)),
        AggExpr::Min(c) => format!("MIN({})", column_expr_name(c)),
        AggExpr::Max(c) => format!("MAX({})", column_expr_name(c)),
        AggExpr::Sum(c) => format!("SUM({})", column_expr_name(c)),
        AggExpr::Avg(c) => format!("AVG({})", column_expr_name(c)),
    }
}

fn column_name(col: &Column) -> &'static str {
    match col {
        Column::Line => "LINE",
        Column::LineNo => "LINENO",
        Column::FileName => "FILENAME",
        Column::FilePath => "FILEPATH",
        Column::FileDir => "FILEDIR",
        Column::FileExt => "FILEEXT",
        Column::FileSize => "FILESIZE",
        Column::Modified => "MODIFIED",
        Column::Created => "CREATED",
        Column::LineCount => "LINECOUNT",
        Column::WordCount => "WORDCOUNT",
        Column::CharCount => "CHARCOUNT",
        Column::Content => "CONTENT",
        Column::FileType => "FILETYPE",
    }
}

fn column_expr_name(expr: &ColumnExpr) -> String {
    let base = column_name(&expr.col).to_string();

    // Wrap with transforms
    let mut name = base;
    for t in &expr.transforms {
        name = match t {
            Transform::Trim(None) => format!("TRIM({name})"),
            Transform::Trim(Some(c)) => format!("TRIM({name}, \"{c}\")"),
            Transform::TrimStart(None) => format!("TRIMSTART({name})"),
            Transform::TrimStart(Some(c)) => format!("TRIMSTART({name}, \"{c}\")"),
            Transform::TrimEnd(None) => format!("TRIMEND({name})"),
            Transform::TrimEnd(Some(c)) => format!("TRIMEND({name}, \"{c}\")"),
            Transform::Upper => format!("UPPER({name})"),
            Transform::Lower => format!("LOWER({name})"),
            Transform::Split(d, i) => format!("SPLIT({name}, \"{d}\", {i})"),
            Transform::Replace(f, t) => format!("REPLACE({name}, \"{f}\", \"{t}\")"),
        };
    }

    // Add slice
    if let Some(ref slice) = expr.slice {
        match slice {
            SliceRange::Index(i) => name = format!("{name}[{i}]"),
            SliceRange::RangeTo(e) => name = format!("{name}[..{e}]"),
            SliceRange::RangeFrom(s) => name = format!("{name}[{s}..]"),
            SliceRange::Range(s, e) => name = format!("{name}[{s}..{e}]"),
        }
    }

    name
}

/// Extract aggregate expressions from a format template string.
/// Looks for patterns like {SUM(LINECOUNT)}, {COUNT(*)}, {AVG(FILESIZE)} etc.
fn extract_template_aggregates(template: &str) -> Vec<AggExpr> {
    let mut aggs = Vec::new();
    let re = regex::Regex::new(r"\{(COUNT\(\*\)|(?:COUNT|SUM|AVG|MIN|MAX)\((\w+)\))").ok();
    if let Some(re) = re {
        for cap in re.captures_iter(template) {
            let full = &cap[1];
            if full == "COUNT(*)" {
                aggs.push(AggExpr::CountAll);
            } else if let Some(col_name) = cap.get(2) {
                let col_name = col_name.as_str();
                let col = match col_name.to_ascii_uppercase().as_str() {
                    "LINE" => Some(Column::Line),
                    "LINENO" => Some(Column::LineNo),
                    "FILENAME" => Some(Column::FileName),
                    "FILEPATH" => Some(Column::FilePath),
                    "FILEDIR" => Some(Column::FileDir),
                    "FILEEXT" => Some(Column::FileExt),
                    "FILESIZE" => Some(Column::FileSize),
                    "MODIFIED" => Some(Column::Modified),
                    "CREATED" => Some(Column::Created),
                    "LINECOUNT" => Some(Column::LineCount),
                    "WORDCOUNT" => Some(Column::WordCount),
                    "CHARCOUNT" => Some(Column::CharCount),
                    "CONTENT" => Some(Column::Content),
                    _ => None,
                };
                if let Some(col) = col {
                    let expr = ColumnExpr { col, slice: None, transforms: vec![] };
                    let func = full.split('(').next().unwrap_or("");
                    match func {
                        "COUNT" => aggs.push(AggExpr::CountCol(expr)),
                        "SUM" => aggs.push(AggExpr::Sum(expr)),
                        "AVG" => aggs.push(AggExpr::Avg(expr)),
                        "MIN" => aggs.push(AggExpr::Min(expr)),
                        "MAX" => aggs.push(AggExpr::Max(expr)),
                        _ => {}
                    }
                }
            }
        }
    }
    aggs
}

fn format_f64(val: f64) -> String {
    if val == val.trunc() {
        format!("{}", val as i64)
    } else {
        format!("{:.2}", val)
    }
}

fn collect_agg_strings(col: &ColumnExpr, rows: &[Row]) -> Vec<String> {
    if is_file_level_column(&col.col) {
        let mut seen = std::collections::HashSet::new();
        rows.iter()
            .filter(|r| seen.insert(r.filepath().to_string()))
            .map(|r| r.get_column_expr(col))
            .collect()
    } else {
        rows.iter().map(|r| r.get_column_expr(col)).collect()
    }
}

fn eval_aggregate(agg: &AggExpr, rows: &[Row]) -> String {
    match agg {
        AggExpr::CountAll => rows.len().to_string(),
        AggExpr::CountCol(col) => {
            let mut seen = std::collections::HashSet::new();
            for r in rows {
                seen.insert(r.get_column_expr(col));
            }
            seen.len().to_string()
        }
        AggExpr::Min(col) => {
            let nums = collect_agg_values(col, rows);
            if !nums.is_empty() {
                format_f64(nums.into_iter().fold(f64::INFINITY, f64::min))
            } else {
                // String comparison fallback (e.g. MODIFIED dates)
                collect_agg_strings(col, rows).into_iter().min().unwrap_or_default()
            }
        }
        AggExpr::Max(col) => {
            let nums = collect_agg_values(col, rows);
            if !nums.is_empty() {
                format_f64(nums.into_iter().fold(f64::NEG_INFINITY, f64::max))
            } else {
                collect_agg_strings(col, rows).into_iter().max().unwrap_or_default()
            }
        }
        AggExpr::Sum(col) => {
            format_f64(collect_agg_values(col, rows).into_iter().sum())
        }
        AggExpr::Avg(col) => {
            let vals = collect_agg_values(col, rows);
            if vals.is_empty() {
                "0".to_string()
            } else {
                format_f64(vals.iter().sum::<f64>() / vals.len() as f64)
            }
        }
    }
}

/// Process escape sequences in separator: \n, \t, \\
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some(other) => { out.push('\\'); out.push(other); }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Output format.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    Default,
    Json,
    Csv,
    Excel,
    Table,
}

fn column_names(items: &[SelectItem]) -> Vec<String> {
    items.iter().map(|item| {
        if let Some(ref alias) = item.alias {
            alias.clone()
        } else {
            // Strip surrounding quotes from format string raw text
            let raw = &item.raw;
            if (raw.starts_with('"') && raw.ends_with('"')) || (raw.starts_with('\'') && raw.ends_with('\'')) {
                raw[1..raw.len()-1].to_string()
            } else {
                raw.clone()
            }
        }
    }).collect()
}

fn render_item(item: &SelectItem, row: &Row) -> String {
    match &item.kind {
        SelectItemKind::Column(c) => row.get_column_expr(c),
        SelectItemKind::Aggregate(agg) => {
            let key = agg_expr_key(agg);
            row.agg_values.get(&key).cloned().unwrap_or_else(|| "0".to_string())
        }
        SelectItemKind::Format(template) => row.format_template(template),
    }
}

/// Format query results for display.
pub fn format_results(result: &QueryResult, query: &Query, format: OutputFormat, no_header: bool) -> String {
    match result {
        QueryResult::Rows(rows) => {
            let SelectClause::Items(items) = &query.select;

            // Build rows as Vec<Vec<String>>
            let data: Vec<Vec<String>> = rows.iter().map(|row| {
                items.iter().map(|item| render_item(item, row)).collect()
            }).collect();

            // Apply DISTINCT
            let data: Vec<Vec<String>> = if query.distinct {
                let mut seen = std::collections::HashSet::new();
                data.into_iter().filter(|row| seen.insert(row.clone())).collect()
            } else {
                data
            };

            let names = column_names(items);
            let has_any_alias = items.iter().any(|i| i.alias.is_some());
            let show_header = has_any_alias && !no_header;

            match format {
                OutputFormat::Json => {
                    let mut out = String::from("[\n");
                    for (i, row) in data.iter().enumerate() {
                        out.push_str("  {");
                        for (j, val) in row.iter().enumerate() {
                            if j > 0 { out.push_str(", "); }
                            let key = &names[j];
                            if val.parse::<f64>().is_ok() {
                                // Number — no quotes on value
                                out.push_str(&format!("\"{key}\": {val}"));
                            } else if val == "true" || val == "false" {
                                out.push_str(&format!("\"{key}\": {val}"));
                            } else {
                                let escaped = val.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\t', "\\t");
                                out.push_str(&format!("\"{key}\": \"{escaped}\""));
                            }
                        }
                        out.push('}');
                        if i + 1 < data.len() { out.push(','); }
                        out.push('\n');
                    }
                    out.push_str("]\n");
                    out
                }
                OutputFormat::Csv => {
                    let mut out = String::new();
                    if show_header {
                        out.push_str(&names.iter().map(|n| csv_escape(n)).collect::<Vec<_>>().join(","));
                        out.push('\n');
                    }
                    for row in &data {
                        out.push_str(&row.iter().map(|v| csv_escape(v)).collect::<Vec<_>>().join(","));
                        out.push('\n');
                    }
                    out
                }
                OutputFormat::Excel => {
                    let mut out = String::new();
                    if show_header {
                        out.push_str(&names.iter().map(|n| excel_escape(n)).collect::<Vec<_>>().join(";"));
                        out.push_str("\r\n");
                    }
                    for row in &data {
                        out.push_str(&row.iter().map(|v| excel_escape(v)).collect::<Vec<_>>().join(";"));
                        out.push_str("\r\n");
                    }
                    out
                }
                OutputFormat::Table => {
                    if data.is_empty() {
                        return String::new();
                    }
                    let ncols = names.len();
                    let mut widths: Vec<usize> = names.iter().map(|n| n.len()).collect();
                    for row in &data {
                        for (j, val) in row.iter().enumerate() {
                            if j < ncols {
                                widths[j] = widths[j].max(val.len());
                            }
                        }
                    }
                    let mut out = String::new();
                    if show_header {
                        for (j, name) in names.iter().enumerate() {
                            if j > 0 { out.push_str("  "); }
                            out.push_str(&format!("{:<width$}", name, width = widths[j]));
                        }
                        out.push('\n');
                        for (j, w) in widths.iter().enumerate() {
                            if j > 0 { out.push_str("  "); }
                            out.push_str(&"-".repeat(*w));
                        }
                        out.push('\n');
                    }
                    for row in &data {
                        for (j, val) in row.iter().enumerate() {
                            if j > 0 { out.push_str("  "); }
                            if j < ncols {
                                out.push_str(&format!("{:<width$}", val, width = widths[j]));
                            }
                        }
                        out.push('\n');
                    }
                    out
                }
                OutputFormat::Default => {
                    let rowsep = query.separator.as_deref().map(unescape).unwrap_or_else(|| "\n".into());
                    let mut out = String::new();
                    if show_header {
                        out.push_str(&names.join(":"));
                        out.push('\n');
                    }
                    let lines: Vec<String> = data.iter().map(|row| row.join(":")).collect();
                    out.push_str(&lines.join(&rowsep));
                    if !lines.is_empty() {
                        out.push('\n');
                    }
                    out
                }
            }
        }
        QueryResult::Numeric(val) => {
            let s = if *val == val.trunc() {
                format!("{}", *val as i64)
            } else {
                format!("{:.2}", val)
            };
            match format {
                OutputFormat::Json => format!("[{{\"result\": \"{}\"}}]\n", s),
                _ => format!("{s}\n"),
            }
        }
    }
}

/// Excel CSV: numbers stay bare, everything else gets ""-quoted.
/// Semicolon delimiter, \r\n line endings.
fn excel_escape(val: &str) -> String {
    // Check if value is numeric (int or float)
    if val.parse::<f64>().is_ok() {
        // Excel uses comma as decimal separator in many locales
        // But semicolon-delimited CSV with dot decimals works universally
        return val.to_string();
    }
    // String: wrap in "", escape inner " as ""
    format!("\"{}\"", val.replace('"', "\"\""))
}

fn csv_escape(val: &str) -> String {
    if val.contains(',') || val.contains('"') || val.contains('\n') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}
