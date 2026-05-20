# fquery

SQL-like queries on files and their contents. Fast, multi-threaded.

```bash
# Find TODOs in source files
fquery 'SELECT * FROM "src/" WHERE LINE CONTAINS "TODO"'

# Count lines per file type with size
fquery 'SELECT FILEEXT, SUM(LINECOUNT) AS Lines, "{SUM(FILESIZE)/1024/1024} MB" AS Size FROM "." GROUP BY FILEEXT ORDER BY SUM(LINECOUNT) DESC'

# Export as JSON
fquery --json 'SELECT FILENAME AS file, LINECOUNT AS lines FROM "src/" WHERE FILEEXT = "rs"'
```

## Install

```bash
cargo build --release
# Binary at target/release/fquery
```

## Features

### SQL Syntax

Full query language: `SELECT`, `FROM`, `WHERE`, `GROUP BY`, `HAVING`, `ORDER BY`, `LIMIT`, `OFFSET`, `DISTINCT`, `SEPARATOR`.

### 17+ Columns

| Column      | Description                    |
| ----------- | ------------------------------ |
| `LINE`      | Line content                   |
| `LINENO`    | Line number                    |
| `FILENAME`  | File name (e.g. `main.rs`)     |
| `FILEPATH`  | Full path                      |
| `FILEDIR`   | Directory part                 |
| `FILEEXT`   | Extension (e.g. `rs`)          |
| `FILESIZE`  | Size in bytes                  |
| `FILETYPE`  | `text` or `binary`             |
| `MODIFIED`  | Last modified (UTC)            |
| `CREATED`   | Creation time (UTC)            |
| `LINECOUNT` | Total lines in file            |
| `WORDCOUNT` | Total words in file            |
| `CHARCOUNT` | Total characters in file       |
| `CONTENT`   | Entire file content            |

### Aggregates

`COUNT(*)`, `COUNT(col)`, `SUM(col)`, `AVG(col)`, `MIN(col)`, `MAX(col)` -- freely mixable with columns.

### String Functions

`TRIM`, `TRIMSTART`, `TRIMEND` (with optional char arg), `UPPER`, `LOWER`, `SPLIT`, `REPLACE`, `LEN`.

### Format Strings

```bash
fquery 'SELECT "{FILENAME}: {FILESIZE/1024} KB" FROM "src/"'
fquery 'SELECT "{FILENAME}: {SUM(LINECOUNT)} lines" FROM "src/" GROUP BY FILENAME'
```

Variables, aggregates, and arithmetic (`+-*/%`) inside `{}`. Escape with `\{`.

### WHERE Conditions

`=`, `!=`, `CONTAINS`, `MATCHES` (regex), `LIKE` (SQL wildcards `%`/`_`), `BETWEEN`, `IN (...)`, `LEN()`, parentheses `(A OR B) AND C`, `FROMFILE("path")`, `NOW()`.

```bash
# Files modified today
fquery 'SELECT FILENAME FROM "src/" WHERE MODIFIED > "2026-05-20 00:00:00"'

# Files larger than 1 MB
fquery 'SELECT FILENAME FROM "." WHERE FILESIZE BETWEEN 1024*1024 AND 1024*1024*10'

# Only Rust and Python
fquery 'SELECT * FROM "src/" WHERE FILEEXT IN ("rs", "py")'
```

### Output Formats

```bash
fquery --json 'SELECT FILENAME AS file, LINECOUNT AS lines FROM "src/"'
fquery --csv 'SELECT FILENAME AS file, FILESIZE AS bytes FROM "src/"'
fquery --excel -o data.csv 'SELECT FILENAME AS file, FILESIZE AS bytes FROM "src/"'
fquery --table 'SELECT FILENAME AS File, FILESIZE AS Bytes, LINECOUNT AS Lines FROM "src/"'
```

Column aliases with `AS`. Headers auto-shown when aliases are present. `--no-header` to suppress.

### Performance

- Multi-threaded file scanning
- File-level query optimization: skips reading file contents when only metadata is needed
- Binary file detection (512-byte null check) -- skipped automatically
- FILESIZE/FILETYPE pre-filtering before any I/O
- Competitive with hand-written bash pipelines on large directory trees

### Piping

```bash
# Read from stdin
echo "data" | fquery 'SELECT * FROM - WHERE LINE CONTAINS "data"'

# Chain queries
fquery 'SELECT FILEPATH FROM "src/" WHERE LINE CONTAINS "error"' | fquery 'SELECT * FROM -'
```

## Full Documentation

```bash
fquery docs
```

## License

MIT
