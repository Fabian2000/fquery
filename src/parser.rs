use crate::error::{FQueryError, Result};

/// A column identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Column {
    Line,
    LineNo,
    FileName,
    FilePath,
    FileDir,
    FileExt,
    FileSize,
    Modified,
    Created,
    LineCount,
    WordCount,
    CharCount,
    Content,
    FileType,
}

/// Optional string slice range on a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SliceRange {
    /// [n] — single character at index
    Index(usize),
    /// [start..end]
    Range(usize, usize),
    /// [..end]
    RangeTo(usize),
    /// [start..]
    RangeFrom(usize),
}

/// Transform applied to a column value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Transform {
    Trim(Option<String>),       // TRIM(col) or TRIM(col, "chars")
    TrimStart(Option<String>),  // TRIMSTART(col) or TRIMSTART(col, "chars")
    TrimEnd(Option<String>),    // TRIMEND(col) or TRIMEND(col, "chars")
    Upper,
    Lower,
    Split(String, usize), // delimiter, field index
    Replace(String, String), // from, to
}

/// A column with optional slice and transforms.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColumnExpr {
    pub col: Column,
    pub slice: Option<SliceRange>,
    pub transforms: Vec<Transform>,
}

/// The kind of a select item.
#[derive(Debug, Clone)]
pub enum SelectItemKind {
    Column(ColumnExpr),
    Aggregate(AggExpr),
    Format(String),
}

/// A single item in the SELECT list, with optional alias.
#[derive(Debug, Clone)]
pub struct SelectItem {
    pub kind: SelectItemKind,
    pub alias: Option<String>,
    pub raw: String,
}

/// What to select.
#[derive(Debug, Clone)]
pub enum SelectClause {
    /// SELECT col1, SUM(col2), "format", COUNT(*), ...
    Items(Vec<SelectItem>),
}

/// An aggregate expression.
#[derive(Debug, Clone)]
pub enum AggExpr {
    /// COUNT(*) — count all rows
    CountAll,
    /// COUNT(col) — count unique values of column
    CountCol(ColumnExpr),
    /// MIN(col)
    Min(ColumnExpr),
    /// MAX(col)
    Max(ColumnExpr),
    /// SUM(col) — numeric columns only
    Sum(ColumnExpr),
    /// AVG(col) — numeric columns only
    Avg(ColumnExpr),
}

/// A single condition in the WHERE clause.
#[derive(Debug, Clone)]
pub enum Condition {
    /// LINE = "exact"
    LineEquals(String),
    /// LINE != "exact"
    LineNotEquals(String),
    /// LINE CONTAINS "substring"
    LineContains(String),
    /// LINE NOT CONTAINS "substring"
    LineNotContains(String),
    /// LINE MATCHES "regex"
    LineMatches(String),
    /// LINE NOT MATCHES "regex"
    LineNotMatches(String),
    /// LINE LIKE "pattern" (SQL wildcards: % = any, _ = single char)
    LineLike(String, bool),
    /// LINENO = N
    LineNoEquals(usize),
    /// LINENO > N
    LineNoGreaterThan(usize),
    /// LINENO < N
    LineNoLessThan(usize),
    /// LINENO >= N
    LineNoGreaterOrEqual(usize),
    /// LINENO <= N
    LineNoLessOrEqual(usize),
    // --- File field conditions ---
    // Each takes (value, negated)
    /// FILENAME|FILEPATH|FILEDIR|FILEEXT = "exact"
    FileFieldEquals(FileField, String, bool),
    /// FILENAME|FILEPATH|FILEDIR|FILEEXT CONTAINS "sub"
    FileFieldContains(FileField, String, bool),
    /// FILENAME|FILEPATH|FILEDIR|FILEEXT MATCHES "regex"
    FileFieldMatches(FileField, String, bool),
    /// FILENAME|FILEPATH|FILEDIR|FILEEXT LIKE "pattern"
    FileFieldLike(FileField, String, bool),
    // --- Generic column expression conditions (for functions in WHERE) ---
    /// LOWER(col) = "value", TRIM(col) CONTAINS "x", etc.
    ExprEquals(ColumnExpr, String, bool),
    ExprContains(ColumnExpr, String, bool),
    ExprMatches(ColumnExpr, String, bool),
    ExprLike(ColumnExpr, String, bool),
    // --- LEN() function ---
    /// LEN(col) <op> N
    LenCmp(ColumnExpr, CmpOp, usize),
    /// FILESIZE BETWEEN N AND M
    FileSizeBetween(u64, u64),
    /// field IN ("a", "b", "c")
    FileFieldIn(FileField, Vec<String>, bool), // field, values, negated
    // --- File metadata conditions ---
    /// FILESIZE <op> N (bytes)
    FileSizeCmp(CmpOp, u64),
    /// MODIFIED <op> "YYYY-MM-DD HH:MM:SS"
    ModifiedCmp(CmpOp, String),
    /// CREATED <op> "YYYY-MM-DD HH:MM:SS"
    CreatedCmp(CmpOp, String),
}

/// Which part of the file path to compare against.
#[derive(Debug, Clone, Copy)]
pub enum FileField {
    /// Just the file name (e.g. "main.rs")
    Name,
    /// Full path (e.g. "/home/user/src/main.rs")
    Path,
    /// Directory part (e.g. "/home/user/src")
    Dir,
    /// Extension only (e.g. "rs")
    Ext,
    /// File type ("text" or "binary")
    Type,
}

/// Comparison operator for numeric/date fields.
#[derive(Debug, Clone, Copy)]
pub enum CmpOp {
    Eq,
    Neq,
    Gt,
    Lt,
    Gte,
    Lte,
}

/// How conditions are combined.
#[derive(Debug, Clone)]
pub enum WhereClause {
    Single(Condition),
    And(Vec<WhereClause>),
    Or(Vec<WhereClause>),
    Not(Box<WhereClause>),
}

/// Optional LIMIT clause.
#[derive(Debug, Clone, Copy)]
pub struct LimitClause {
    pub count: usize,
}

/// A parsed query.
#[derive(Debug, Clone)]
pub struct Query {
    pub select: SelectClause,
    pub distinct: bool,
    pub separator: Option<String>,
    pub from: String,
    pub where_clause: Option<WhereClause>,
    pub group_by: Option<GroupBy>,
    pub having: Option<HavingClause>,
    pub order_by: Option<OrderBy>,
    pub limit: Option<LimitClause>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone)]
pub enum OrderByExpr {
    Column(ColumnExpr),
    Aggregate(AggExpr),
}

#[derive(Debug, Clone)]
pub struct OrderBy {
    pub expr: OrderByExpr,
    pub desc: bool,
}

/// GROUP BY uses a ColumnExpr — any column, with optional transforms/slicing.
pub type GroupBy = ColumnExpr;

/// A single HAVING condition.
#[derive(Debug, Clone)]
pub struct HavingCondition {
    pub agg: AggExpr,
    pub op: CmpOp,
    pub value: f64,
}

/// HAVING clause — one or more conditions combined with AND.
#[derive(Debug, Clone)]
pub struct HavingClause {
    pub conditions: Vec<HavingCondition>,
}

/// Tokenizer for the SQL-like syntax.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Keyword(String),
    StringLiteral(String),
    Number(usize),
    Comma,
    Star,
    LParen,
    RParen,
    LBracket,
    RBracket,
    DotDot,
    Eq,
    Neq,
    Gt,
    Lt,
    Gte,
    Lte,
}

fn tokenize(input: &str) -> Result<(Vec<Token>, Vec<usize>)> {
    let mut tokens = Vec::new();
    let mut positions = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let token_start = i;
        match chars[i] {
            c if c.is_whitespace() => {
                i += 1;
            }
            '"' | '\'' => {
                let quote = chars[i];
                i += 1;
                let mut s = String::new();
                while i < len && chars[i] != quote {
                    if chars[i] == '\\' && i + 1 < len {
                        match chars[i + 1] {
                            c if c == quote => { s.push(c); i += 2; }
                            '\\' => { s.push('\\'); i += 2; }
                            'n' => { s.push('\n'); i += 2; }
                            't' => { s.push('\t'); i += 2; }
                            _ => { s.push(chars[i]); i += 1; }
                        }
                    } else {
                        s.push(chars[i]);
                        i += 1;
                    }
                }
                if i >= len {
                    return Err(FQueryError::Parse(format!("unterminated string literal starting at position {}", token_start + 1)));
                }
                positions.push(token_start); tokens.push(Token::StringLiteral(s));
                i += 1;
            }
            '(' => {
                positions.push(token_start); tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                positions.push(token_start); tokens.push(Token::RParen);
                i += 1;
            }
            '[' => {
                positions.push(token_start); tokens.push(Token::LBracket);
                i += 1;
            }
            ']' => {
                positions.push(token_start); tokens.push(Token::RBracket);
                i += 1;
            }
            '.' if i + 1 < len && chars[i + 1] == '.' => {
                positions.push(token_start); tokens.push(Token::DotDot);
                i += 2;
            }
            '*' => {
                positions.push(token_start); tokens.push(Token::Star);
                i += 1;
            }
            ',' => {
                positions.push(token_start); tokens.push(Token::Comma);
                i += 1;
            }
            '!' if i + 1 < len && chars[i + 1] == '=' => {
                positions.push(token_start); tokens.push(Token::Neq);
                i += 2;
            }
            '>' if i + 1 < len && chars[i + 1] == '=' => {
                positions.push(token_start); tokens.push(Token::Gte);
                i += 2;
            }
            '<' if i + 1 < len && chars[i + 1] == '=' => {
                positions.push(token_start); tokens.push(Token::Lte);
                i += 2;
            }
            '=' => {
                positions.push(token_start); tokens.push(Token::Eq);
                i += 1;
            }
            '>' => {
                positions.push(token_start); tokens.push(Token::Gt);
                i += 1;
            }
            '<' => {
                positions.push(token_start); tokens.push(Token::Lt);
                i += 1;
            }
            c if c.is_ascii_digit() => {
                let start = i;
                while i < len && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                let n = s
                    .parse::<usize>()
                    .map_err(|e| FQueryError::Parse(format!("invalid number: {e}")))?;
                positions.push(token_start); tokens.push(Token::Number(n));
            }
            c if c.is_alphanumeric() || c == '_' || c == '.' || c == '/' || c == '~' || c == '-' => {
                let start = i;
                while i < len
                    && (chars[i].is_alphanumeric()
                        || chars[i] == '_'
                        || chars[i] == '.'
                        || chars[i] == '/'
                        || chars[i] == '~'
                        || chars[i] == '-'
                        || chars[i] == '?')
                {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                positions.push(token_start); tokens.push(Token::Keyword(s));
            }
            other => {
                return Err(FQueryError::Parse(format!("unexpected character '{other}' at position {}", i + 1)));
            }
        }
    }

    Ok((tokens, positions))
}

struct Parser {
    tokens: Vec<Token>,
    positions: Vec<usize>,
    input: String,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>, positions: Vec<usize>, input: String) -> Self {
        Self { tokens, positions, input, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn current_position(&self) -> usize {
        self.positions.get(self.pos).copied().unwrap_or(0)
    }

    fn advance(&mut self) -> Result<&Token> {
        let tok = self.tokens.get(self.pos).ok_or_else(|| {
            FQueryError::Parse("unexpected end of query".into())
        })?;
        self.pos += 1;
        Ok(tok)
    }

    fn expect_keyword(&mut self, expected: &str) -> Result<()> {
        let tok = self.advance()?.clone();
        match &tok {
            Token::Keyword(k) if k.eq_ignore_ascii_case(expected) => Ok(()),
            other => Err(FQueryError::Parse(format!(
                "expected '{expected}', got {other:?}"
            ))),
        }
    }

    fn is_keyword(&self, expected: &str) -> bool {
        matches!(self.peek(), Some(Token::Keyword(k)) if k.eq_ignore_ascii_case(expected))
    }

    fn parse_query(&mut self) -> Result<Query> {
        self.expect_keyword("SELECT")?;
        // Optional: SELECT SEPARATOR("...") col1, col2
        let separator = if self.is_keyword("SEPARATOR") {
            self.advance()?;
            if !matches!(self.peek(), Some(Token::LParen)) {
                return Err(FQueryError::Parse("expected '(' after SEPARATOR".into()));
            }
            self.advance()?;
            let val = self.expect_string()?;
            if !matches!(self.peek(), Some(Token::RParen)) {
                return Err(FQueryError::Parse("expected ')' after SEPARATOR value".into()));
            }
            self.advance()?;
            Some(val)
        } else {
            None
        };
        // DISTINCT flag (works with any select type)
        let distinct = if self.is_keyword("DISTINCT") {
            self.advance()?;
            true
        } else {
            false
        };

        let select = self.parse_select()?;
        self.expect_keyword("FROM")?;
        let from = self.parse_from()?;
        let where_clause = if self.is_keyword("WHERE") {
            self.advance()?;
            Some(self.parse_where()?)
        } else {
            None
        };

        // GROUP BY any column expression
        let group_by = if self.is_keyword("GROUP") {
            self.advance()?;
            self.expect_keyword("BY")?;
            Some(self.parse_single_column()?)
        } else {
            None
        };

        // HAVING agg_expr <op> N [AND agg_expr <op> N ...]
        let having = if self.is_keyword("HAVING") {
            self.advance()?;
            let mut conditions = vec![self.parse_having_condition()?];
            while self.is_keyword("AND") {
                self.advance()?;
                conditions.push(self.parse_having_condition()?);
            }
            Some(HavingClause { conditions })
        } else {
            None
        };

        // ORDER BY col|aggregate [ASC|DESC]
        let order_by = if self.is_keyword("ORDER") {
            self.advance()?;
            self.expect_keyword("BY")?;
            let item = self.parse_select_item_kind()?;
            let expr = match item {
                SelectItemKind::Column(c) => OrderByExpr::Column(c),
                SelectItemKind::Aggregate(a) => OrderByExpr::Aggregate(a),
                SelectItemKind::Format(_) => return Err(FQueryError::Parse(
                    "ORDER BY does not support format strings".into()
                )),
            };
            let desc = if self.is_keyword("DESC") {
                self.advance()?;
                true
            } else {
                if self.is_keyword("ASC") {
                    self.advance()?;
                }
                false
            };
            Some(OrderBy { expr, desc })
        } else {
            None
        };

        // Parse LIMIT and OFFSET in any order
        let mut limit = None;
        let mut offset = None;
        for _ in 0..2 {
            if limit.is_none() && self.is_keyword("LIMIT") {
                self.advance()?;
                let tok = self.advance()?.clone();
                limit = Some(match tok {
                    Token::Number(n) => LimitClause { count: n },
                    other => {
                        return Err(FQueryError::Parse(format!(
                            "expected number after LIMIT, got {other:?}"
                        )))
                    }
                });
            } else if offset.is_none() && self.is_keyword("OFFSET") {
                self.advance()?;
                let tok = self.advance()?.clone();
                offset = Some(match tok {
                    Token::Number(n) => n,
                    other => {
                        return Err(FQueryError::Parse(format!(
                            "expected number after OFFSET, got {other:?}"
                        )))
                    }
                });
            }
        }

        Ok(Query {
            select,
            distinct,
            separator,
            from,
            where_clause,
            group_by,
            having,
            order_by,
            limit,
            offset,
        })
    }

    fn try_parse_column(keyword: &str) -> Option<Column> {
        let upper = keyword.to_ascii_uppercase();
        match upper.as_str() {
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
            "FILETYPE" => Some(Column::FileType),
            _ => None,
        }
    }


    fn parse_single_column(&mut self) -> Result<ColumnExpr> {
        let tok = self.peek().cloned();

        // Check for function-style: TRIM(col), UPPER(col), LOWER(col),
        // SPLIT(col, "delim", idx), REPLACE(col, "from", "to")
        if let Some(Token::Keyword(k)) = &tok {
            let upper = k.to_ascii_uppercase();
            match upper.as_str() {
                "TRIM" | "TRIMSTART" | "TRIMEND" | "UPPER" | "LOWER" => {
                    self.advance()?;
                    if !matches!(self.peek(), Some(Token::LParen)) {
                        return Err(FQueryError::Parse(format!("expected '(' after {upper}")));
                    }
                    self.advance()?;
                    let mut inner = self.parse_single_column()?;
                    // Optional second arg for trim chars: TRIM(col, "chars")
                    let trim_chars = if matches!(upper.as_str(), "TRIM" | "TRIMSTART" | "TRIMEND")
                        && matches!(self.peek(), Some(Token::Comma))
                    {
                        self.advance()?;
                        Some(self.expect_string()?)
                    } else {
                        None
                    };
                    if !matches!(self.peek(), Some(Token::RParen)) {
                        return Err(FQueryError::Parse(format!("expected ')' after {upper}(..)")));
                    }
                    self.advance()?;
                    let transform = match upper.as_str() {
                        "TRIM" => Transform::Trim(trim_chars),
                        "TRIMSTART" => Transform::TrimStart(trim_chars),
                        "TRIMEND" => Transform::TrimEnd(trim_chars),
                        "UPPER" => Transform::Upper,
                        "LOWER" => Transform::Lower,
                        _ => unreachable!(),
                    };
                    inner.transforms.push(transform);
                    self.parse_optional_slice(&mut inner)?;
                    return Ok(inner);
                }
                "SPLIT" => {
                    self.advance()?;
                    if !matches!(self.peek(), Some(Token::LParen)) {
                        return Err(FQueryError::Parse("expected '(' after SPLIT".into()));
                    }
                    self.advance()?;
                    let mut inner = self.parse_single_column()?;
                    if !matches!(self.peek(), Some(Token::Comma)) {
                        return Err(FQueryError::Parse("expected ',' after SPLIT(col".into()));
                    }
                    self.advance()?;
                    let delim = self.expect_string()?;
                    if !matches!(self.peek(), Some(Token::Comma)) {
                        return Err(FQueryError::Parse("expected ',' after SPLIT(col, delim".into()));
                    }
                    self.advance()?;
                    let idx = self.expect_number()?;
                    if !matches!(self.peek(), Some(Token::RParen)) {
                        return Err(FQueryError::Parse("expected ')' after SPLIT(col, delim, idx".into()));
                    }
                    self.advance()?;
                    inner.transforms.push(Transform::Split(delim, idx));
                    self.parse_optional_slice(&mut inner)?;
                    return Ok(inner);
                }
                "REPLACE" => {
                    self.advance()?;
                    if !matches!(self.peek(), Some(Token::LParen)) {
                        return Err(FQueryError::Parse("expected '(' after REPLACE".into()));
                    }
                    self.advance()?;
                    let mut inner = self.parse_single_column()?;
                    if !matches!(self.peek(), Some(Token::Comma)) {
                        return Err(FQueryError::Parse("expected ',' after REPLACE(col".into()));
                    }
                    self.advance()?;
                    let from = self.expect_string()?;
                    if !matches!(self.peek(), Some(Token::Comma)) {
                        return Err(FQueryError::Parse("expected ',' after REPLACE(col, from".into()));
                    }
                    self.advance()?;
                    let to = self.expect_string()?;
                    if !matches!(self.peek(), Some(Token::RParen)) {
                        return Err(FQueryError::Parse("expected ')' after REPLACE(col, from, to".into()));
                    }
                    self.advance()?;
                    inner.transforms.push(Transform::Replace(from, to));
                    self.parse_optional_slice(&mut inner)?;
                    return Ok(inner);
                }
                _ => {}
            }
        }

        // Plain column
        let tok = self.advance()?.clone();
        let col = match &tok {
            Token::Keyword(k) => {
                Self::try_parse_column(k).ok_or_else(|| {
                    FQueryError::Parse(format!("unknown column: {k}"))
                })?
            }
            other => return Err(FQueryError::Parse(format!(
                "expected column name, got {other:?}"
            ))),
        };
        // Optional [range]
        let slice = if matches!(self.peek(), Some(Token::LBracket)) {
            self.advance()?;
            let range = self.parse_slice_range()?;
            if !matches!(self.peek(), Some(Token::RBracket)) {
                return Err(FQueryError::Parse("expected ']' after range".into()));
            }
            self.advance()?;
            Some(range)
        } else {
            None
        };
        Ok(ColumnExpr { col, slice, transforms: vec![] })
    }

    fn parse_optional_slice(&mut self, expr: &mut ColumnExpr) -> Result<()> {
        if matches!(self.peek(), Some(Token::LBracket)) {
            self.advance()?;
            let range = self.parse_slice_range()?;
            if !matches!(self.peek(), Some(Token::RBracket)) {
                return Err(FQueryError::Parse("expected ']' after range".into()));
            }
            self.advance()?;
            expr.slice = Some(range);
        }
        Ok(())
    }

    fn parse_slice_range(&mut self) -> Result<SliceRange> {
        // [..end] or [..] (invalid) or [start] or [start..] or [start..end]
        if matches!(self.peek(), Some(Token::DotDot)) {
            // [..end]
            self.advance()?;
            let end = self.expect_number()?;
            return Ok(SliceRange::RangeTo(end));
        }
        let start = self.expect_number()?;
        if matches!(self.peek(), Some(Token::DotDot)) {
            self.advance()?;
            // [start..] or [start..end]
            if matches!(self.peek(), Some(Token::Number(_))) {
                let end = self.expect_number()?;
                Ok(SliceRange::Range(start, end))
            } else {
                Ok(SliceRange::RangeFrom(start))
            }
        } else {
            // [n] — single index
            Ok(SliceRange::Index(start))
        }
    }

    /// Extract the raw input text between two token positions.
    fn raw_text_between(&self, start_token: usize, end_token: usize) -> String {
        let start_char = self.positions.get(start_token).copied().unwrap_or(0);
        let end_char = if end_token < self.positions.len() {
            self.positions[end_token]
        } else {
            self.input.len()
        };
        self.input[start_char..end_char].trim().to_string()
    }

    fn parse_select_item(&mut self) -> Result<SelectItem> {
        let start = self.pos;
        let kind = self.parse_select_item_kind()?;
        let raw = self.raw_text_between(start, self.pos);
        let alias = if self.is_keyword("AS") {
            self.advance()?;
            let tok = self.advance()?.clone();
            match tok {
                Token::Keyword(s) => Some(s),
                Token::StringLiteral(s) => Some(s),
                other => return Err(FQueryError::Parse(format!(
                    "expected alias name after AS, got {other:?}"
                ))),
            }
        } else {
            None
        };
        Ok(SelectItem { kind, alias, raw })
    }

    fn parse_select_item_kind(&mut self) -> Result<SelectItemKind> {
        // COUNT(*)
        if self.is_keyword("COUNT") {
            self.advance()?;
            if !matches!(self.peek(), Some(Token::LParen)) {
                return Err(FQueryError::Parse("expected '(' after COUNT".into()));
            }
            self.advance()?;
            if matches!(self.peek(), Some(Token::Star)) {
                self.advance()?;
                if !matches!(self.peek(), Some(Token::RParen)) {
                    return Err(FQueryError::Parse("expected ')' after COUNT(*".into()));
                }
                self.advance()?;
                return Ok(SelectItemKind::Aggregate(AggExpr::CountAll));
            }
            let col = self.parse_single_column()?;
            if !matches!(self.peek(), Some(Token::RParen)) {
                return Err(FQueryError::Parse("expected ')' after COUNT(col".into()));
            }
            self.advance()?;
            return Ok(SelectItemKind::Aggregate(AggExpr::CountCol(col)));
        }

        // MIN/MAX/AVG/SUM(col)
        if let Some(Token::Keyword(k)) = self.peek() {
            let upper = k.to_ascii_uppercase();
            if matches!(upper.as_str(), "MIN" | "MAX" | "AVG" | "SUM") {
                self.advance()?;
                if !matches!(self.peek(), Some(Token::LParen)) {
                    return Err(FQueryError::Parse(format!("expected '(' after {upper}")));
                }
                self.advance()?;
                let col = self.parse_single_column()?;
                if !matches!(self.peek(), Some(Token::RParen)) {
                    return Err(FQueryError::Parse(format!("expected ')' after {upper}(col")));
                }
                self.advance()?;
                let expr = match upper.as_str() {
                    "MIN" => AggExpr::Min(col),
                    "MAX" => AggExpr::Max(col),
                    "AVG" => AggExpr::Avg(col),
                    "SUM" => AggExpr::Sum(col),
                    _ => unreachable!(),
                };
                return Ok(SelectItemKind::Aggregate(expr));
            }
        }

        // Format string
        if matches!(self.peek(), Some(Token::StringLiteral(_))) {
            let tok = self.advance()?.clone();
            if let Token::StringLiteral(s) = tok {
                return Ok(SelectItemKind::Format(s));
            }
        }

        // Regular column
        Ok(SelectItemKind::Column(self.parse_single_column()?))
    }

    fn parse_select(&mut self) -> Result<SelectClause> {
        // SELECT *
        if matches!(self.peek(), Some(Token::Star)) {
            self.advance()?;
            return Ok(SelectClause::Items(vec![
                SelectItem { kind: SelectItemKind::Column(ColumnExpr { col: Column::FilePath, slice: None, transforms: vec![] }), alias: None, raw: "FILEPATH".into() },
                SelectItem { kind: SelectItemKind::Column(ColumnExpr { col: Column::LineNo, slice: None, transforms: vec![] }), alias: None, raw: "LINENO".into() },
                SelectItem { kind: SelectItemKind::Column(ColumnExpr { col: Column::Line, slice: None, transforms: vec![] }), alias: None, raw: "LINE".into() },
            ]));
        }

        // SELECT item1, item2, ... (columns and aggregates freely mixed)
        let mut items = vec![self.parse_select_item()?];
        while matches!(self.peek(), Some(Token::Comma)) {
            self.advance()?;
            items.push(self.parse_select_item()?);
        }
        Ok(SelectClause::Items(items))
    }

    fn parse_from(&mut self) -> Result<String> {
        let tok = self.advance()?.clone();
        match tok {
            Token::Star => Ok("*".into()),
            Token::Keyword(s) => Ok(s),
            Token::StringLiteral(s) => Ok(s),
            other => Err(FQueryError::Parse(format!(
                "expected file path or pattern after FROM, got {other:?}"
            ))),
        }
    }

    fn parse_where(&mut self) -> Result<WhereClause> {
        let left = self.parse_condition_or_not()?;

        match self.peek() {
            Some(Token::Keyword(k)) if k.eq_ignore_ascii_case("AND") => {
                let mut parts = vec![left];
                while self.is_keyword("AND") {
                    self.advance()?;
                    parts.push(self.parse_condition_or_not()?);
                }
                Ok(WhereClause::And(parts))
            }
            Some(Token::Keyword(k)) if k.eq_ignore_ascii_case("OR") => {
                let mut parts = vec![left];
                while self.is_keyword("OR") {
                    self.advance()?;
                    parts.push(self.parse_condition_or_not()?);
                }
                Ok(WhereClause::Or(parts))
            }
            _ => Ok(left),
        }
    }

    fn parse_condition_or_not(&mut self) -> Result<WhereClause> {
        if self.is_keyword("NOT") {
            self.advance()?;
            let inner = self.parse_condition_or_not()?;
            return Ok(WhereClause::Not(Box::new(inner)));
        }
        // Parenthesized sub-expression
        if matches!(self.peek(), Some(Token::LParen)) {
            self.advance()?;
            let inner = self.parse_where()?;
            if !matches!(self.peek(), Some(Token::RParen)) {
                return Err(FQueryError::Parse("expected ')' after grouped condition".into()));
            }
            self.advance()?;
            return Ok(inner);
        }
        let cond = self.parse_condition()?;
        Ok(WhereClause::Single(cond))
    }

    fn parse_expr_condition(&mut self, expr: ColumnExpr) -> Result<Condition> {
        let op = self.advance()?.clone();
        match &op {
            Token::Eq => {
                let val = self.expect_string()?;
                Ok(Condition::ExprEquals(expr, val, false))
            }
            Token::Neq => {
                let val = self.expect_string()?;
                Ok(Condition::ExprEquals(expr, val, true))
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("CONTAINS") => {
                let val = self.expect_string()?;
                Ok(Condition::ExprContains(expr, val, false))
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("MATCHES") => {
                let val = self.expect_string()?;
                Ok(Condition::ExprMatches(expr, val, false))
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("LIKE") => {
                let val = self.expect_string()?;
                Ok(Condition::ExprLike(expr, val, false))
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("NOT") => {
                let next = self.advance()?.clone();
                match &next {
                    Token::Keyword(kk) if kk.eq_ignore_ascii_case("CONTAINS") => {
                        let val = self.expect_string()?;
                        Ok(Condition::ExprContains(expr, val, true))
                    }
                    Token::Keyword(kk) if kk.eq_ignore_ascii_case("MATCHES") => {
                        let val = self.expect_string()?;
                        Ok(Condition::ExprMatches(expr, val, true))
                    }
                    Token::Keyword(kk) if kk.eq_ignore_ascii_case("LIKE") => {
                        let val = self.expect_string()?;
                        Ok(Condition::ExprLike(expr, val, true))
                    }
                    other => Err(FQueryError::Parse(format!(
                        "expected CONTAINS, MATCHES, or LIKE after NOT, got {other:?}"
                    ))),
                }
            }
            other => Err(FQueryError::Parse(format!(
                "expected operator after expression, got {other:?}"
            ))),
        }
    }

    fn parse_condition(&mut self) -> Result<Condition> {
        // Check for function-wrapped columns: LOWER(col), UPPER(col), TRIM(col), etc.
        if let Some(Token::Keyword(k)) = self.peek() {
            let upper = k.to_ascii_uppercase();
            if matches!(upper.as_str(), "LOWER" | "UPPER" | "TRIM" | "TRIMSTART" | "TRIMEND" | "SPLIT" | "REPLACE") {
                let expr = self.parse_single_column()?;
                return self.parse_expr_condition(expr);
            }
        }

        let field = self.advance()?.clone();
        match &field {
            Token::Keyword(k) if k.eq_ignore_ascii_case("LINE") => {
                self.parse_line_condition()
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("LINENO") => {
                self.parse_lineno_condition()
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("FILENAME") => {
                self.parse_file_field_condition(FileField::Name)
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("FILEPATH") => {
                self.parse_file_field_condition(FileField::Path)
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("FILEDIR") => {
                self.parse_file_field_condition(FileField::Dir)
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("FILEEXT") => {
                self.parse_file_field_condition(FileField::Ext)
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("FILETYPE") => {
                self.parse_file_field_condition(FileField::Type)
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("LEN") => {
                return self.parse_len_condition();
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("FILESIZE") => {
                self.parse_filesize_condition()
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("MODIFIED") => {
                self.parse_date_condition(true)
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("CREATED") => {
                self.parse_date_condition(false)
            }
            other => Err(FQueryError::Parse(format!(
                "expected field (LINE, LINENO, FILENAME, FILEPATH, FILEDIR, FILEEXT, FILESIZE, FILETYPE, MODIFIED, CREATED), got {other:?}"
            ))),
        }
    }

    fn parse_line_condition(&mut self) -> Result<Condition> {
        let op = self.advance()?.clone();
        match &op {
            Token::Eq => {
                let val = self.expect_string()?;
                Ok(Condition::LineEquals(val))
            }
            Token::Neq => {
                let val = self.expect_string()?;
                Ok(Condition::LineNotEquals(val))
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("CONTAINS") => {
                let val = self.expect_string()?;
                Ok(Condition::LineContains(val))
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("LIKE") => {
                let val = self.expect_string()?;
                Ok(Condition::LineLike(val, false))
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("NOT") => {
                let next = self.advance()?.clone();
                match &next {
                    Token::Keyword(kk) if kk.eq_ignore_ascii_case("CONTAINS") => {
                        let val = self.expect_string()?;
                        Ok(Condition::LineNotContains(val))
                    }
                    Token::Keyword(kk) if kk.eq_ignore_ascii_case("MATCHES") => {
                        let val = self.expect_string()?;
                        Ok(Condition::LineNotMatches(val))
                    }
                    Token::Keyword(kk) if kk.eq_ignore_ascii_case("LIKE") => {
                        let val = self.expect_string()?;
                        Ok(Condition::LineLike(val, true))
                    }
                    other => Err(FQueryError::Parse(format!(
                        "expected CONTAINS, MATCHES, or LIKE after NOT, got {other:?}"
                    ))),
                }
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("MATCHES") => {
                let val = self.expect_string()?;
                Ok(Condition::LineMatches(val))
            }
            other => Err(FQueryError::Parse(format!(
                "expected operator after LINE, got {other:?}"
            ))),
        }
    }

    fn parse_in_list(&mut self, field: FileField, negated: bool) -> Result<Condition> {
        // IN ("a", "b", "c")
        if !matches!(self.peek(), Some(Token::LParen)) {
            return Err(FQueryError::Parse("expected '(' after IN".into()));
        }
        self.advance()?;
        let mut values = vec![self.expect_string()?];
        while matches!(self.peek(), Some(Token::Comma)) {
            self.advance()?;
            values.push(self.expect_string()?);
        }
        if !matches!(self.peek(), Some(Token::RParen)) {
            return Err(FQueryError::Parse("expected ')' after IN list".into()));
        }
        self.advance()?;
        Ok(Condition::FileFieldIn(field, values, negated))
    }

    fn parse_having_condition(&mut self) -> Result<HavingCondition> {
        let agg_item = self.parse_select_item_kind()?;
        let agg = match agg_item {
            SelectItemKind::Aggregate(a) => a,
            _ => return Err(FQueryError::Parse(
                "HAVING requires an aggregate function (COUNT, SUM, AVG, MIN, MAX)".into()
            )),
        };
        let op = self.parse_cmp_op()?;
        let value = self.parse_numeric_expr()?;
        Ok(HavingCondition { agg, op, value })
    }

    fn parse_len_condition(&mut self) -> Result<Condition> {
        // LEN(col_expr) <op> N
        if !matches!(self.peek(), Some(Token::LParen)) {
            return Err(FQueryError::Parse("expected '(' after LEN".into()));
        }
        self.advance()?;
        let col = self.parse_single_column()?;
        if !matches!(self.peek(), Some(Token::RParen)) {
            return Err(FQueryError::Parse("expected ')' after LEN(col".into()));
        }
        self.advance()?;
        let op = self.parse_cmp_op()?;
        let num = self.parse_numeric_expr()? as usize;
        Ok(Condition::LenCmp(col, op, num))
    }

    fn parse_lineno_condition(&mut self) -> Result<Condition> {
        let op = self.advance()?.clone();
        let num = self.expect_number()?;
        match &op {
            Token::Eq => Ok(Condition::LineNoEquals(num)),
            Token::Gt => Ok(Condition::LineNoGreaterThan(num)),
            Token::Lt => Ok(Condition::LineNoLessThan(num)),
            Token::Gte => Ok(Condition::LineNoGreaterOrEqual(num)),
            Token::Lte => Ok(Condition::LineNoLessOrEqual(num)),
            other => Err(FQueryError::Parse(format!(
                "expected comparison operator after LINENO, got {other:?}"
            ))),
        }
    }

    fn parse_file_field_condition(&mut self, field: FileField) -> Result<Condition> {
        let op = self.advance()?.clone();
        match &op {
            Token::Eq => {
                let val = self.expect_string()?;
                Ok(Condition::FileFieldEquals(field, val, false))
            }
            Token::Neq => {
                let val = self.expect_string()?;
                Ok(Condition::FileFieldEquals(field, val, true))
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("CONTAINS") => {
                let val = self.expect_string()?;
                Ok(Condition::FileFieldContains(field, val, false))
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("MATCHES") => {
                let val = self.expect_string()?;
                Ok(Condition::FileFieldMatches(field, val, false))
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("LIKE") => {
                let val = self.expect_string()?;
                Ok(Condition::FileFieldLike(field, val, false))
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("IN") => {
                self.parse_in_list(field, false)
            }
            Token::Keyword(k) if k.eq_ignore_ascii_case("NOT") => {
                let next = self.advance()?.clone();
                match &next {
                    Token::Keyword(kk) if kk.eq_ignore_ascii_case("CONTAINS") => {
                        let val = self.expect_string()?;
                        Ok(Condition::FileFieldContains(field, val, true))
                    }
                    Token::Keyword(kk) if kk.eq_ignore_ascii_case("MATCHES") => {
                        let val = self.expect_string()?;
                        Ok(Condition::FileFieldMatches(field, val, true))
                    }
                    Token::Keyword(kk) if kk.eq_ignore_ascii_case("LIKE") => {
                        let val = self.expect_string()?;
                        Ok(Condition::FileFieldLike(field, val, true))
                    }
                    Token::Keyword(kk) if kk.eq_ignore_ascii_case("IN") => {
                        self.parse_in_list(field, true)
                    }
                    other => Err(FQueryError::Parse(format!(
                        "expected CONTAINS, MATCHES, LIKE, or IN after NOT, got {other:?}"
                    ))),
                }
            }
            other => Err(FQueryError::Parse(format!(
                "expected =, !=, CONTAINS, MATCHES, LIKE, IN, or NOT after file field, got {other:?}"
            ))),
        }
    }

    fn parse_cmp_op(&mut self) -> Result<CmpOp> {
        let tok = self.advance()?.clone();
        match &tok {
            Token::Eq => Ok(CmpOp::Eq),
            Token::Neq => Ok(CmpOp::Neq),
            Token::Gt => Ok(CmpOp::Gt),
            Token::Lt => Ok(CmpOp::Lt),
            Token::Gte => Ok(CmpOp::Gte),
            Token::Lte => Ok(CmpOp::Lte),
            other => Err(FQueryError::Parse(format!(
                "expected comparison operator, got {other:?}"
            ))),
        }
    }

    fn parse_numeric_expr(&mut self) -> Result<f64> {
        let tok = self.advance()?.clone();
        let mut val = match tok {
            Token::Number(n) => n as f64,
            other => return Err(FQueryError::Parse(format!(
                "expected number, got {other:?}"
            ))),
        };
        // Chain arithmetic: 1024*1024, 1024/2, etc.
        loop {
            match self.peek() {
                Some(Token::Star) => {
                    self.advance()?;
                    let n = self.expect_number()? as f64;
                    val *= n;
                }
                // / is part of keyword chars, so check for a keyword that starts with /
                // Actually / gets consumed into keywords. We need to handle this differently.
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_filesize_condition(&mut self) -> Result<Condition> {
        // FILESIZE BETWEEN N AND M
        if self.is_keyword("BETWEEN") {
            self.advance()?;
            let low = self.parse_numeric_expr()? as u64;
            self.expect_keyword("AND")?;
            let high = self.parse_numeric_expr()? as u64;
            return Ok(Condition::FileSizeBetween(low, high));
        }
        let op = self.parse_cmp_op()?;
        let val = self.parse_numeric_expr()?;
        Ok(Condition::FileSizeCmp(op, val as u64))
    }

    /// Parse a date value — either a string literal, FROMFILE(), or NOW()
    fn expect_date_value(&mut self) -> Result<String> {
        if self.is_keyword("NOW") {
            self.advance()?;
            // Optional ()
            if matches!(self.peek(), Some(Token::LParen)) {
                self.advance()?;
                if matches!(self.peek(), Some(Token::RParen)) {
                    self.advance()?;
                }
            }
            return Ok(now_utc());
        }
        self.expect_string()
    }

    fn parse_date_condition(&mut self, is_modified: bool) -> Result<Condition> {
        let op = self.parse_cmp_op()?;
        let val = self.expect_date_value()?;
        if is_modified {
            Ok(Condition::ModifiedCmp(op, val))
        } else {
            Ok(Condition::CreatedCmp(op, val))
        }
    }

    fn expect_string(&mut self) -> Result<String> {
        // Support FROMFILE("path") — reads first line of file, trimmed
        if self.is_keyword("FROMFILE") {
            self.advance()?;
            if !matches!(self.peek(), Some(Token::LParen)) {
                return Err(FQueryError::Parse("expected '(' after FROMFILE".into()));
            }
            self.advance()?;
            let path_tok = self.advance()?.clone();
            let path = match path_tok {
                Token::StringLiteral(s) => s,
                other => {
                    return Err(FQueryError::Parse(format!(
                        "expected file path string in FROMFILE(), got {other:?}"
                    )))
                }
            };
            if !matches!(self.peek(), Some(Token::RParen)) {
                return Err(FQueryError::Parse("expected ')' after FROMFILE(path".into()));
            }
            self.advance()?;
            let content = std::fs::read_to_string(&path).map_err(|e| {
                FQueryError::Io {
                    path: std::path::PathBuf::from(&path),
                    source: e,
                }
            })?;
            // Use first line, trimmed
            let val = content.lines().next().unwrap_or("").trim().to_string();
            return Ok(val);
        }

        let tok = self.advance()?.clone();
        match tok {
            Token::StringLiteral(s) => Ok(s),
            other => Err(FQueryError::Parse(format!(
                "expected string literal or FROMFILE(), got {other:?}"
            ))),
        }
    }

    fn expect_number(&mut self) -> Result<usize> {
        let tok = self.advance()?.clone();
        match tok {
            Token::Number(n) => Ok(n),
            other => Err(FQueryError::Parse(format!(
                "expected number, got {other:?}"
            ))),
        }
    }
}

/// Get current UTC time as "YYYY-MM-DD HH:MM:SS".
fn now_utc() -> String {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
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
    format!("{y:04}-{m:02}-{d:02} {hours:02}:{minutes:02}:{seconds:02}")
}

/// Parse a query string into a structured Query.
pub fn parse(input: &str) -> Result<Query> {
    let (tokens, positions) = tokenize(input)?;
    let mut parser = Parser::new(tokens, positions, input.to_string());
    let query = parser.parse_query()?;
    if let Some(tok) = parser.peek() {
        let pos = parser.current_position() + 1;
        return Err(FQueryError::Parse(format!(
            "unexpected token at position {pos}: {tok:?}"
        )));
    }
    Ok(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_select_all() {
        let q = parse(r#"SELECT * FROM "test.txt""#).expect("should parse");
        assert!(matches!(q.select, SelectClause::Items(_)));
        assert_eq!(q.from, "test.txt");
    }

    #[test]
    fn test_where_contains() {
        let q = parse(r#"SELECT * FROM "*.rs" WHERE LINE CONTAINS "fn ""#).expect("should parse");
        assert!(q.where_clause.is_some());
    }

    #[test]
    fn test_limit() {
        let q = parse(r#"SELECT * FROM * WHERE LINE = "x" LIMIT 10"#).expect("should parse");
        assert_eq!(q.limit.map(|l| l.count), Some(10));
    }

    #[test]
    fn test_offset_limit_any_order() {
        let q = parse(r#"SELECT * FROM * OFFSET 5 LIMIT 10"#).expect("should parse");
        assert_eq!(q.limit.map(|l| l.count), Some(10));
        assert_eq!(q.offset, Some(5));
    }

    #[test]
    fn test_distinct() {
        let q = parse(r#"SELECT DISTINCT FILENAME FROM "src/""#).expect("should parse");
        assert!(q.distinct);
    }

    #[test]
    fn test_separator() {
        let q = parse(r#"SELECT SEPARATOR(";") FILENAME FROM "src/""#).expect("should parse");
        assert_eq!(q.separator.as_deref(), Some(";"));
    }

    #[test]
    fn test_group_by() {
        let q = parse(r#"SELECT FILEEXT, COUNT(*) FROM "src/" GROUP BY FILEEXT"#).expect("should parse");
        assert!(q.group_by.is_some());
    }

    #[test]
    fn test_order_by_desc() {
        let q = parse(r#"SELECT * FROM "src/" ORDER BY FILESIZE DESC"#).expect("should parse");
        assert!(q.order_by.is_some());
        assert!(q.order_by.as_ref().map(|o| o.desc).unwrap_or(false));
    }

    #[test]
    fn test_having() {
        let q = parse(r#"SELECT FILEEXT FROM "src/" GROUP BY FILEEXT HAVING COUNT(*) > 5 AND SUM(LINECOUNT) > 100"#).expect("should parse");
        assert!(q.having.is_some());
        assert_eq!(q.having.as_ref().map(|h| h.conditions.len()), Some(2));
    }

    #[test]
    fn test_format_string() {
        let q = parse(r#"SELECT "{FILENAME}:{LINENO}" FROM "src/""#).expect("should parse");
        let SelectClause::Items(items) = &q.select;
        assert!(matches!(&items[0], SelectItem { kind: SelectItemKind::Format(_), .. }));
    }

    #[test]
    fn test_mixed_columns_and_format() {
        let q = parse(r#"SELECT FILENAME, "{FILESIZE/1024} KB" FROM "src/""#).expect("should parse");
        let SelectClause::Items(items) = &q.select;
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], SelectItem { kind: SelectItemKind::Column(_), .. }));
        assert!(matches!(&items[1], SelectItem { kind: SelectItemKind::Format(_), .. }));
    }

    #[test]
    fn test_aggregate_mixed() {
        let q = parse(r#"SELECT FILEEXT, COUNT(*), SUM(LINECOUNT) FROM "src/" GROUP BY FILEEXT"#).expect("should parse");
        let SelectClause::Items(items) = &q.select;
        assert_eq!(items.len(), 3);
        assert!(matches!(&items[1], SelectItem { kind: SelectItemKind::Aggregate(AggExpr::CountAll), .. }));
    }

    #[test]
    fn test_in_list() {
        let q = parse(r#"SELECT * FROM "src/" WHERE FILEEXT IN ("rs", "py", "js")"#).expect("should parse");
        assert!(q.where_clause.is_some());
    }

    #[test]
    fn test_between() {
        let q = parse(r#"SELECT * FROM "src/" WHERE FILESIZE BETWEEN 1024 AND 1024*1024"#).expect("should parse");
        assert!(q.where_clause.is_some());
    }

    #[test]
    fn test_parenthesized_where() {
        let q = parse(r#"SELECT * FROM "src/" WHERE (LINE CONTAINS "a" OR LINE CONTAINS "b") AND FILEEXT = "rs""#).expect("should parse");
        assert!(q.where_clause.is_some());
    }

    #[test]
    fn test_like() {
        parse(r#"SELECT * FROM "src/" WHERE LINE LIKE "hello%""#).expect("should parse");
    }

    #[test]
    fn test_len() {
        parse(r#"SELECT * FROM "src/" WHERE LEN(FILEEXT) < 4"#).expect("should parse");
    }

    #[test]
    fn test_case_insensitive() {
        parse(r#"select * from "src/" where line contains "test" limit 5"#).expect("should parse");
    }

    #[test]
    fn test_column_functions() {
        parse(r#"SELECT TRIM(SPLIT(LINE, ",", 0))[..10] FROM "src/""#).expect("should parse");
    }

    #[test]
    fn test_escaped_quotes() {
        let q = parse(r#"SELECT * FROM "src/" WHERE LINE CONTAINS "he said \"hello\"""#).expect("should parse");
        assert!(q.where_clause.is_some());
    }

    #[test]
    fn test_stdin() {
        let q = parse(r#"SELECT * FROM -"#).expect("should parse");
        assert_eq!(q.from, "-");
    }

    #[test]
    fn test_trailing_tokens_error() {
        let result = parse(r#"SELECT * FROM "src/" BLAH"#);
        assert!(result.is_err());
    }

    #[test]
    fn test_arithmetic_in_filesize() {
        parse(r#"SELECT * FROM "src/" WHERE FILESIZE > 1024*1024"#).expect("should parse");
    }
}
