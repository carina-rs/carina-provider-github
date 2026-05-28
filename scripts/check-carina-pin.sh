#!/bin/bash
# Guard against a stale / inconsistent carina-core git pin.
#
# Provider repos pin carina-core (+ carina-plugin-sdk +
# carina-provider-protocol) by git `rev` in their Cargo.toml files.
# Two recurring failure modes this catches:
#
#   1. INCONSISTENT pin — different revs across the workspace's
#      Cargo.toml files. Every crate that depends on carina-core (or
#      another carina-rs/carina crate) must pin the SAME rev; a
#      mismatch links two carina-core crates and silently breaks type
#      identity (documented hazard, carina-rs/carina-provider-awscc#255).
#      Always a bug. Hard-fail.
#
#   2. STALE pin — the pinned rev predates a carina-core fix this
#      provider's correctness now depends on. `.carina-core-min-rev`
#      records the minimum required carina-core commit; the pinned rev
#      must be that commit or a descendant of it. A stale pin ships
#      known phantom-diff bugs (e.g. the schema-aware detail-row
#      renderer / List<StringEnum> reconciliation, carina#3073/#3075).
#      Hard-fail so staleness is visible and testable, not silent.
#
# Exit 0 = OK, non-zero = problem (message explains the fix).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

CARINA_GIT="https://github.com/carina-rs/carina"
MIN_REV_FILE=".carina-core-min-rev"

fail() { echo "::error::$*" >&2; echo "carina-pin check FAILED: $*" >&2; exit 1; }

# --- 1. Collect every carina-* git pin across the workspace ----------
# (No `mapfile` / process-substitution-into-array: keep this portable
#  to bash 3.2, which ships on macOS — and never let a missing builtin
#  silently no-op a CI gate.)
# Every Cargo.toml line declaring a carina-rs/carina git dependency.
carina_dep_lines="$(grep -rn 'git = "https://github.com/carina-rs/carina"' \
  --include=Cargo.toml . 2>/dev/null || true)"

[ -n "$carina_dep_lines" ] || fail "no carina git deps found in any Cargo.toml"

# A carina git dep MUST be `rev`-pinned. A floating `branch`/`tag`
# (or any carina dep with no `rev`) is itself a failure — it would
# make the build non-reproducible and silently dodge this guard.
# Whitespace around `=` is TOML-optional, so match it loosely.
REV_RE='rev[[:space:]]*=[[:space:]]*"([0-9a-fA-F]+)"'
unpinned="$(printf '%s\n' "$carina_dep_lines" \
  | grep -Ev "$REV_RE" || true)"
if [ -n "$unpinned" ]; then
    echo "carina git dep(s) not rev-pinned:" >&2
    printf '%s\n' "$unpinned" >&2
    fail "carina dependency must be pinned by exact 'rev = \"<hash>\"' (no branch/tag)"
fi

pin_lines="$carina_dep_lines"
revs="$(printf '%s\n' "$pin_lines" \
  | sed -E "s/.*${REV_RE}.*/\\1/")"
pin_count="$(printf '%s\n' "$pin_lines" | wc -l | tr -d ' ')"

# --- 2. Consistency: all pins must be the SAME rev ------------------
uniq_revs="$(printf '%s\n' "$revs" | sort -u)"
if [ "$(printf '%s\n' "$uniq_revs" | wc -l | tr -d ' ')" -ne 1 ]; then
    echo "Pinned carina revs are NOT consistent across Cargo.toml:" >&2
    printf '%s\n' "$pin_lines" >&2
    fail "inconsistent carina-core pin (mixed revs link two carina-core crates; pin them all to the same rev)"
fi
PINNED_REV="$uniq_revs"
echo "carina-pin: all ${pin_count} pins on rev ${PINNED_REV}"

# --- 3. Staleness: pinned rev must be >= .carina-core-min-rev -------
if [ ! -f "$MIN_REV_FILE" ]; then
    fail "$MIN_REV_FILE missing (record the minimum required carina-core rev there)"
fi
MIN_REV="$(tr -d '[:space:]' < "$MIN_REV_FILE")"
[ -n "$MIN_REV" ] || fail "$MIN_REV_FILE is empty"

# Ancestry check needs a carina clone. CI clones it; locally we use an
# adjacent ../carina checkout if present, else a temp shallow-ish clone.
CARINA_DIR=""
if [ -n "${CARINA_CORE_DIR:-}" ] && [ -d "${CARINA_CORE_DIR}/.git" ]; then
    CARINA_DIR="$CARINA_CORE_DIR"
elif [ -d "../carina/.git" ]; then
    CARINA_DIR="$(cd ../carina && pwd)"
else
    CARINA_DIR="$(mktemp -d)/carina"
    echo "carina-pin: cloning carina to verify rev ancestry..."
    git clone --quiet --filter=blob:none "$CARINA_GIT" "$CARINA_DIR"
fi

git -C "$CARINA_DIR" fetch --quiet origin "$MIN_REV" "$PINNED_REV" 2>/dev/null || \
    git -C "$CARINA_DIR" fetch --quiet origin 2>/dev/null || true

for r in "$MIN_REV" "$PINNED_REV"; do
    git -C "$CARINA_DIR" cat-file -e "${r}^{commit}" 2>/dev/null || \
        fail "carina rev ${r} not found in ${CARINA_GIT} (bad rev in pin or $MIN_REV_FILE)"
done

# pinned must be MIN_REV itself or a descendant of it.
if git -C "$CARINA_DIR" merge-base --is-ancestor "$MIN_REV" "$PINNED_REV"; then
    echo "carina-pin OK: ${PINNED_REV} is at/after required ${MIN_REV}"
    exit 0
fi

fail "STALE carina-core pin: ${PINNED_REV} predates the required minimum ${MIN_REV} \
(this provider depends on a newer carina-core; bump every carina-* rev in the \
Cargo.toml files to ${MIN_REV} or a later carina main commit, then update \
${MIN_REV_FILE} if a newer minimum is intended)"
