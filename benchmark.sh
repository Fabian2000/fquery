#!/bin/bash
set -e

DIR="/home/local_projects/"
FQUERY="./target/debug/fquery"

echo "=== FQUERY ==="
time $FQUERY 'SELECT ".{FILEEXT}\t\t{SUM(LINECOUNT)} lines\t\t{SUM(FILESIZE)/1024/1024/1024} GB" FROM "'"$DIR"'" WHERE FILEDIR NOT CONTAINS "target" AND FILEDIR NOT CONTAINS ".git" AND FILENAME != ".gitignore" AND FILEEXT != "" AND FILEDIR NOT CONTAINS "obj" AND FILEDIR NOT CONTAINS "bin" AND FILEDIR NOT CONTAINS ".claude" AND FILEDIR NOT CONTAINS ".vscode" AND FILETYPE = "text" AND LEN(FILEEXT) < 4 AND FILEEXT NOT MATCHES "[0-9]+|CL|CUF|BSD" AND FILESIZE > 1024*1024 GROUP BY LOWER(FILEEXT) ORDER BY FILEEXT LIMIT 10' | sort > /tmp/fquery_out.txt

echo ""
echo "=== BASH/AWK ==="
time find "$DIR" \
  -not -path "*/target/*" \
  -not -path "*/.git/*" \
  -not -path "*/obj/*" \
  -not -path "*/bin/*" \
  -not -path "*/.claude/*" \
  -not -path "*/.vscode/*" \
  -not -name ".gitignore" \
  -type f \
  -size +1M \
  -print0 \
| xargs -0 -P4 file --mime-type 2>/dev/null \
| grep "text/" \
| awk -F: '{print $1}' \
| while IFS= read -r f; do
    ext="${f##*.}"
    ext=$(echo "$ext" | tr '[:upper:]' '[:lower:]')
    if [ "${#ext}" -lt 4 ] && ! echo "$ext" | grep -qE '^([0-9]+|cl|cuf|bsd)$'; then
      lines=$(wc -l < "$f" 2>/dev/null || echo 0)
      size=$(stat -c%s "$f" 2>/dev/null || echo 0)
      echo "$ext $lines $size"
    fi
  done \
| awk '{l[$1]+=$2; s[$1]+=$3} END{for(e in l) printf ".%s\t\t%d lines\t\t%.2f GB\n", e, l[e], s[e]/1024/1024/1024}' \
| sort \
| head -10 > /tmp/bash_out.txt

cat /tmp/bash_out.txt

echo ""
echo "=== DIFF ==="
diff /tmp/fquery_out.txt /tmp/bash_out.txt && echo "IDENTICAL" || echo "DIFFERENT (see above)"
