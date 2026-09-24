#!/usr/bin/env bash
# stack_report.sh — measured language composition (what GitHub Linguist sees).
# Uses find so it needs no globstar; counts every tracked source tree.
cd "$(dirname "$0")/.."
count() { # language, find-expr
  local lang="$1"; shift
  local files lines
  files=$(find "$@" -type f 2>/dev/null | grep -v '^\./\.git' | wc -l)
  lines=$(find "$@" -type f 2>/dev/null | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}')
  [ "${lines:-0}" -gt 0 ] && printf "%-12s %8s %8s\n" "$lang" "$lines" "$files"
}
printf "%-12s %8s %8s\n" "language" "lines" "files"
count Rust    src -name '*.rs'
count C       runtime -name '*.c'
count C++     runtime -name '*.cpp'
count Python  bootstrap -name '*.py'
count Shell   scripts -name '*.sh'
count HTML    docs -name '*.html'
count CSS     docs -name '*.css'
count JS      web -name '*.js'
count TS      web -name '*.ts'
count Operon  std -name '*.op'
count Operon2 tests -name '*.op'
count Operon3 examples -name '*.op'
count Operon4 apps -name '*.op'
count Operon5 tools -name '*.op'
echo "---"
echo "total Operon (.op) lines: $(find std tests examples apps tools -name '*.op' 2>/dev/null | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}')"
