#!/bin/sh
# Downloads the real books reader-core's tests read into target/fixtures (or $READER_FIXTURES).
set -eu
cd "$(dirname "$0")/.."
dir="${READER_FIXTURES:-target/fixtures}"
mkdir -p "$dir"
fetch() {
  [ -s "$dir/$1" ] || curl -fsSL -o "$dir/$1" "$2"
  echo "$dir/$1"
}
fetch pride-and-prejudice.epub https://www.gutenberg.org/ebooks/1342.epub3.images
fetch rfc2616.pdf https://www.rfc-editor.org/rfc/pdfrfc/rfc2616.txt.pdf
