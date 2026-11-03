#!/bin/bash
# Generate GitHub provider documentation from its validated resource schemas.
# Usage (from project root): ./scripts/generate-docs.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DOCS_DIR="$PROJECT_ROOT/generated-docs/github"

cd "$PROJECT_ROOT"

# Build successfully before deleting the existing generated documentation.
cargo build --locked -p carina-provider-github-docgen --bin docgen --quiet
TARGET_DIR=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
DOCGEN_BIN="$TARGET_DIR/debug/docgen"
if [ ! -x "$DOCGEN_BIN" ]; then
    echo "ERROR: docgen binary not found at $DOCGEN_BIN" >&2
    exit 1
fi

rm -rf "$DOCS_DIR"
"$DOCGEN_BIN" --output-dir "$DOCS_DIR"

# Add Starlight frontmatter, preserving the schema's DSL name as the title.
# The docs bundle restores this title as an H1 when packaging the page.
for OUTPUT_FILE in "$DOCS_DIR"/*/*.md; do
    [ -f "$OUTPUT_FILE" ] || continue
    DSL_NAME_FULL=$(sed -n '1s/^# //p' "$OUTPUT_FILE")
    if [ -z "$DSL_NAME_FULL" ]; then
        echo "ERROR: generated page has no title: $OUTPUT_FILE" >&2
        exit 1
    fi
    FRONTMATTER_TMPFILE=$(mktemp)
    {
        printf '%s\n' '---' "title: \"$DSL_NAME_FULL\"" '---' ''
        sed '1{/^# /d;}' "$OUTPUT_FILE"
    } > "$FRONTMATTER_TMPFILE"
    mv "$FRONTMATTER_TMPFILE" "$OUTPUT_FILE"
done

echo "Generated documentation in $DOCS_DIR"
