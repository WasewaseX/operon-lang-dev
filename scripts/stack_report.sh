#!/usr/bin/env bash
# stack_report.sh — measured language composition (what GitHub Linguist would see).
cd "$(dirname "$0")/.."
printf "%-12s %8s %8s\n" "language" "lines" "files"
for pair in "Rust:src/**/*.rs" "C:runtime/*.c" "C++:runtime/*.cpp" "Python:bootstrap/*.py" "Operon:std/*.op" "Operon2:tools/*.op" "Operon3:tests/*.op" "Operon4:examples/*.op" "Operon5:apps/**/*.op" "Shell:scripts/*.sh" "HTML:docs/*.html" "CSS:docs/*.css" "JavaScript:web/playground/*.js" "TypeScript:web/playground/*.ts"; do
  lang="${pair%%:*}"; pat="${pair#*:}"
  total=0
  for f in $(ls $pat 2>/dev/null); do n=$(wc -l < "$f"); total=$((total + n)); done
  [ "$total" -gt 0 ] && printf "%-12s %8d %8s\n" "$lang" "$total" "$(ls $pat 2>/dev/null | wc -l)"
done
echo "---"
echo "total .op (Operon) lines:"
cat std/*.op tools/*.op tests/*.op examples/*.op apps/genomelab/*.op 2>/dev/null | wc -l
