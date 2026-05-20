#!/bin/bash
set -e

DIR="/home/local_projects/"
FQUERY="./target/release/fquery"

echo "=== FQUERY ==="
time $FQUERY 'SELECT ".{FILEEXT}\t\t{SUM(LINECOUNT)} lines\t\t{SUM(FILESIZE)/1024/1024/1024} GB" FROM "'"$DIR"'" WHERE FILETYPE = "text" AND FILESIZE > 1024*1024 AND LEN(FILEEXT) > 0 AND LEN(FILEEXT) < 4 AND FILEDIR NOT CONTAINS "target" AND FILEDIR NOT CONTAINS ".git" AND FILEDIR NOT CONTAINS "obj" AND FILEDIR NOT CONTAINS "bin" AND FILEDIR NOT CONTAINS ".claude" AND FILEDIR NOT CONTAINS ".vscode" AND FILEEXT NOT IN ("CL", "CUF", "BSD") AND FILEEXT NOT MATCHES "^[0-9]+$" GROUP BY LOWER(FILEEXT) ORDER BY FILEEXT LIMIT 10' > /tmp/fquery_result.txt 2>&1

cat /tmp/fquery_result.txt
echo ""

echo "=== BASH ==="
time find "$DIR" \
  -not -path "*/target/*" \
  -not -path "*/.git/*" \
  -not -path "*/obj/*" \
  -not -path "*/bin/*" \
  -not -path "*/.claude/*" \
  -not -path "*/.vscode/*" \
  -type f \
  -size +1M \
  -print0 \
| while IFS= read -r -d '' f; do
    # Get extension
    base="${f##*/}"
    case "$base" in
      *.*) ext="${base##*.}" ;;
      *) continue ;;  # no extension, skip
    esac

    # LEN(ext) > 0 AND < 4
    if [ -z "$ext" ] || [ "${#ext}" -ge 4 ]; then
      continue
    fi

    # Lowercase for grouping
    ext_lower=$(echo "$ext" | tr '[:upper:]' '[:lower:]')

    # NOT IN ("CL", "CUF", "BSD")
    case "$ext" in
      CL|CUF|BSD) continue ;;
    esac

    # NOT MATCHES "^[0-9]+$"
    if echo "$ext" | grep -qE '^[0-9]+$'; then
      continue
    fi

    # Binary check: use file command (matches fquery's null-byte check better)
    mime=$(file -b --mime-encoding "$f" 2>/dev/null)
    case "$mime" in
      binary|unknown-8bit) continue ;;
    esac

    # Count lines (awk counts like Rust's .lines() — includes last line without \n)
    lines=$(awk 'END{print NR}' "$f" 2>/dev/null || echo 0)
    size=$(stat -c%s "$f" 2>/dev/null || echo 0)
    echo "$ext_lower $lines $size"
  done \
| awk '{
    l[$1]+=$2
    s[$1]+=$3
  }
  END {
    n=0
    for(e in l) {
      keys[n++]=e
    }
    # Sort keys
    for(i=0;i<n;i++)
      for(j=i+1;j<n;j++)
        if(keys[i]>keys[j]) { t=keys[i]; keys[i]=keys[j]; keys[j]=t }
    # Print first 10
    for(i=0;i<n && i<10;i++) {
      e=keys[i]
      printf ".%s\t\t%d lines\t\t%.2f GB\n", e, l[e], s[e]/1024/1024/1024
    }
  }' > /tmp/bash_result.txt 2>&1

cat /tmp/bash_result.txt
echo ""

echo "=== DIFF ==="
diff /tmp/fquery_result.txt /tmp/bash_result.txt && echo "IDENTICAL" || echo "DIFFERENT"
