mod engine;
mod error;
mod parser;

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;

const HELP_TEXT: &str = r##"
fquery — SQL-like queries on files and their contents

SYNTAX:
  SELECT [SEPARATOR("sep")] [DISTINCT] <items> FROM <source>
    [WHERE <conditions>]
    [GROUP BY <column>]
    [HAVING <aggregate> <op> <N>]
    [ORDER BY <column> [ASC|DESC]]
    [LIMIT N] [OFFSET N]

COLUMNS (freely mixable with aggregates and format strings, comma-separated):
  *                   shorthand for FILEPATH, LINENO, LINE
  LINE                line content
  LINENO              line number
  FILENAME            file name (e.g. "main.rs")
  FILEPATH            full path (e.g. "/home/user/src/main.rs")
  FILEDIR             directory (e.g. "/home/user/src")
  FILEEXT             extension (e.g. "rs")
  FILESIZE            file size in bytes
  FILETYPE            "text" or "binary"
  MODIFIED            last modified (UTC, "YYYY-MM-DD HH:MM:SS")
  CREATED             creation time (UTC)
  LINECOUNT           total lines in file
  WORDCOUNT           total words in file
  CHARCOUNT           total characters in file
  CONTENT             entire file content

AGGREGATES (usable in SELECT, format strings, HAVING, ORDER BY):
  COUNT(*)            count all matching rows
  COUNT(col)          count unique values
  SUM(col)            sum numeric values
  AVG(col)            average numeric values
  MIN(col)            minimum value
  MAX(col)            maximum value
  File-level columns (FILESIZE, LINECOUNT, etc.) are auto-deduplicated.

COLUMN FUNCTIONS (nestable, usable in SELECT and WHERE via LEN):
  TRIM(col)                     strip whitespace
  TRIM(col, "/")                strip specific characters
  TRIMSTART(col)                strip leading whitespace
  TRIMSTART(col, "/")           strip leading specific chars
  TRIMEND(col)                  strip trailing whitespace
  TRIMEND(col, ".")             strip trailing specific chars
  UPPER(col)                    uppercase
  LOWER(col)                    lowercase
  SPLIT(col, "delim", index)    split and pick field (0-based, multi-char ok)
  REPLACE(col, "from", "to")   string replacement
  LEN(col) <op> N               string length in WHERE conditions

SLICING (on any column or function result):
  col[5]          single character at index
  col[..10]       first 10 characters
  col[5..]        from index 5 onwards
  col[5..10]      characters 5 to 10

FORMAT STRINGS (mixable with columns: SELECT FILENAME, "{FILESIZE/1024} KB"):
  Variables: {LINE}, {LINENO}, {FILENAME}, {FILEPATH}, {FILEDIR}, {FILEEXT},
             {FILESIZE}, {FILETYPE}, {MODIFIED}, {CREATED},
             {LINECOUNT}, {WORDCOUNT}, {CHARCOUNT}, {CONTENT}
  Aggregates: {COUNT(*)}, {SUM(LINECOUNT)}, {AVG(FILESIZE)}, {MIN(col)}, {MAX(col)}
  Arithmetic: {FILESIZE/1024}, {SUM(FILESIZE)/1024/1024} (+-*/% chainable)
  Escape: \{ \} for literal braces (important for JSON-containing files!)
  Escape: \" for quotes inside strings, \\ for literal backslash

MODIFIERS (after SELECT, before columns):
  SEPARATOR(";")      set row separator (default: newline), supports \n \t \\
  DISTINCT            deduplicate output rows

SOURCE:
  "file.txt"          single file
  "*.rs"              glob pattern
  "src/"              directory (scanned recursively)
  "src/**/*.rs"       recursive glob
  *                   all files in current directory
  -                   read from stdin (for piping)

WHERE CONDITIONS:
  LINE = "text"                    exact match
  LINE != "text"                   not equal
  LINE CONTAINS "text"             substring match
  LINE NOT CONTAINS "text"         exclude substring
  LINE MATCHES "regex"             regex match (full regex syntax)
  LINE NOT MATCHES "regex"         exclude regex
  LINE LIKE "hello%"               SQL wildcards (% = any, _ = one char)
  LINE NOT LIKE "%test%"           exclude pattern
  LINENO = 5                       line number comparison (= != > < >= <=)
  FILENAME/FILEPATH/FILEDIR/FILEEXT/FILETYPE
    = != CONTAINS MATCHES LIKE     all string operators on all file fields
  FILESIZE > 1024                  file size in bytes
  FILESIZE BETWEEN 1024 AND 1024*1024  range check (arithmetic ok)
  FILEEXT IN ("rs", "py", "js")    match any value in list
  FILEEXT NOT IN ("tmp", "log")    exclude values in list
  MODIFIED > "2025-01-01 00:00:00" last modified time (UTC)
  MODIFIED > NOW()                 compare against current UTC time
  CREATED < "2025-06-01 00:00:00"  creation time (UTC)
  NOW() usable wherever a date string is expected
  LEN(col) < 80                    string length (works with TRIM, SPLIT, etc.)
  FROMFILE("path")                 read value from file (usable as any string)

  Arithmetic in values: FILESIZE > 1024*1024, LEN(LINE) > 2*40
  All keywords are case-insensitive. Escape quotes with \" inside strings.

COMBINATORS:
  ... AND ...         both conditions must match
  ... OR ...          either condition must match
  NOT ...             negate a condition
  (A OR B) AND C      parentheses for grouping

GROUP BY / HAVING / ORDER BY:
  GROUP BY col                     group rows (any column, functions, slices)
  HAVING COUNT(*) > 10             filter groups by aggregate condition
  HAVING COUNT(*) > 5 AND SUM(LINECOUNT) > 100   multiple conditions
  ORDER BY col [ASC|DESC]          sort results (numeric-aware)
  ORDER BY SUM(LINECOUNT) DESC     sort by aggregate

OPTIONS:
  -t, --threads N     number of threads (default: all CPU cores)
  -o, --output FILE   write output to file instead of stdout

OUTPUT FORMATS:
  --json              JSON array (numbers auto-detected, no quotes on numeric values)
  --csv               CSV (comma-separated)
  --excel             Excel CSV (semicolon, \r\n, strings quoted, numbers bare)
  --table             aligned table with column headers
  --no-header         suppress header row

COLUMN ALIASES:
  SELECT col AS name          name columns for headers
  Without AS: header uses the original expression (e.g. LOWER(FILEEXT), SUM(LINECOUNT))
  Header auto-shown when any AS alias is present; --no-header overrides

EXAMPLES:
  fquery 'SELECT * FROM "src/" WHERE LINE CONTAINS "TODO"'
  fquery 'SELECT FILENAME, COUNT(*), SUM(LINECOUNT) FROM "src/" GROUP BY FILENAME'
  fquery 'SELECT FILEEXT, "{SUM(LINECOUNT)} lines", "{SUM(FILESIZE)/1024/1024} MB" FROM "." GROUP BY FILEEXT HAVING SUM(LINECOUNT) > 100 ORDER BY FILEEXT'
  fquery 'SELECT SEPARATOR("; ") DISTINCT FILENAME FROM "src/" WHERE FILEEXT = "rs"'
  fquery 'SELECT DISTINCT "{FILENAME}: {LINECOUNT} lines, {WORDCOUNT} words" FROM "src/"'
  fquery 'SELECT LINE FROM "log.txt" WHERE LINE NOT LIKE "#%" AND LEN(TRIM(LINE)) > 0 LIMIT 5 OFFSET 10'
  fquery 'SELECT TRIM(SPLIT(LINE, ",", 0))[..20] FROM "data.csv"'
  fquery 'SELECT * FROM "src/" WHERE FILETYPE = "text" AND FILEDIR NOT CONTAINS ".git"'
  fquery 'SELECT * FROM - WHERE LINE CONTAINS "error"'
  fquery 'SELECT LINE FROM "src/" WHERE LINE CONTAINS FROMFILE("secret.txt")'
  fquery -t 2 'SELECT FILEEXT, SUM(LINECOUNT) FROM "/" WHERE FILETYPE = "text" GROUP BY FILEEXT'
  fquery 'SELECT MIN(MODIFIED) AS Oldest, MAX(MODIFIED) AS Newest FROM "src/" WHERE FILEEXT = "rs"'
  fquery --json 'SELECT FILEEXT AS ext, SUM(LINECOUNT) AS lines FROM "src/" GROUP BY FILEEXT'
  fquery --csv 'SELECT FILENAME AS file, FILESIZE AS bytes FROM "src/"'
  fquery --table 'SELECT FILENAME AS File, FILESIZE AS Bytes, LINECOUNT AS Lines FROM "src/"'
"##;

#[derive(Parser)]
#[command(name = "fquery", about = "SQL-like queries on files and their contents")]
struct Cli {
    /// The query to execute, or "docs" for full syntax reference.
    query: String,

    /// Number of threads (default: number of CPU cores)
    #[arg(short = 't', long = "threads")]
    threads: Option<usize>,

    /// Write output to file instead of stdout
    #[arg(short = 'o', long = "output")]
    output: Option<String>,

    /// Output as JSON array
    #[arg(long = "json")]
    json: bool,

    /// Output as CSV
    #[arg(long = "csv")]
    csv: bool,

    /// Output as Excel-compatible CSV (semicolon, \r\n, quoted strings)
    #[arg(long = "excel")]
    excel: bool,

    /// Output as aligned table
    #[arg(long = "table")]
    table: bool,

    /// Suppress header row (for CSV/table output)
    #[arg(long = "no-header")]
    no_header: bool,
}

fn run() -> error::Result<()> {
    let cli = Cli::parse();

    if cli.query.eq_ignore_ascii_case("docs") {
        print!("{HELP_TEXT}");
        return Ok(());
    }

    if let Some(threads) = cli.threads {
        let threads = threads.max(1);
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .ok(); // ignore if already initialized
    }

    let input = cli.query;

    let format = if cli.json {
        engine::OutputFormat::Json
    } else if cli.excel {
        engine::OutputFormat::Excel
    } else if cli.csv {
        engine::OutputFormat::Csv
    } else if cli.table {
        engine::OutputFormat::Table
    } else {
        engine::OutputFormat::Default
    };

    let query = parser::parse(&input)?;
    let result = engine::execute(&query)?;
    let output = engine::format_results(&result, &query, format, cli.no_header);

    if let Some(path) = cli.output {
        std::fs::write(&path, output.as_bytes()).map_err(|e| error::FQueryError::Io {
            path: std::path::PathBuf::from(path),
            source: e,
        })?;
    } else {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(output.as_bytes());
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
