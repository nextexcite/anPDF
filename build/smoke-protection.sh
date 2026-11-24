#!/bin/sh
# Round-trip the synthetic one-page fixture using the packaged executable.
set -eu
anpdf=$1
root=$2
input=$3
output=$4
qpdf=${ANPDF_QPDF:-qpdf}

umask 077
work=$(mktemp -d "$root/$output/protection.XXXXXX")
trap 'rm -rf "$work"' EXIT
relative=${work#"$root/"}
relative=${relative#./}
printf '%s\n' 'synthetic-protection-test' > "$work/password"

"$anpdf" encrypt --root "$root" "$input" -o "$relative/locked.pdf" --password-file "$work/password" </dev/null >/dev/null
"$qpdf" --requires-password "$work/locked.pdf"
"$anpdf" decrypt --root "$root" "$relative/locked.pdf" -o "$relative/open.pdf" --password-file "$work/password" </dev/null >/dev/null
"$qpdf" --check "$work/open.pdf" >/dev/null
test "$("$qpdf" --show-npages "$work/open.pdf")" = 1
"$anpdf" extract-text --root "$root" "$relative/open.pdf" 1 </dev/null > "$work/text.json"
grep -q 'PAGE-ONE' "$work/text.json"
printf '%s\n' 'Packaged password protection passed: locked output requires a password; decrypted copy retains PAGE-ONE'
