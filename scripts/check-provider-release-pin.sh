#!/usr/bin/env bash
# Keep every provider-release action and workflow on the same immutable revision.
set -euo pipefail

fail() { printf '::error::%s\n' "$*" >&2; exit 1; }

workflows_dir=${1:-.github/workflows}
[[ -d "$workflows_dir" ]] || fail "Missing workflows directory: $workflows_dir"
repo_root=$(cd "$workflows_dir/../.." && pwd)
shopt -s nullglob
workflows=("$workflows_dir"/*.yml "$workflows_dir"/*.yaml)
[[ ${#workflows[@]} -gt 0 ]] || fail "No .yml or .yaml workflows in $workflows_dir"

awk -v ci="$workflows_dir/ci.yml" -v release="$workflows_dir/release.yml" '
    FNR == 1 { depth = 0 }
    /^[[:space:]]*(#|$)/ { next }
    {
        # Track indentation so a step cannot stand in for a reusable workflow job.
        indent = match($0, /[^[:space:]]/)
        while (depth && indent <= indents[depth]) depth--
        indents[++depth] = indent
        keys[depth] = $1
    }
    /^[[:space:]]*(-[[:space:]]+)?uses:[[:space:]]*["\047]?carina-rs\/provider-release\// {
        use = $0
        sub(/^[[:space:]]*(-[[:space:]]+)?uses:[[:space:]]*/, "", use)
        sub(/[[:space:]]+#.*$/, "", use)
        sub(/[[:space:]]+$/, "", use)
        gsub(/^["\047]|["\047]$/, "", use)
        ref = use
        sub(/^[^@]*@/, "", ref)
        if (!(ref in refs)) count++
        refs[ref] = refs[ref] sprintf("    %s:%d\n", FILENAME, FNR)
        if (length(ref) != 40 || ref !~ /^[0-9a-fA-F]+$/) invalid = 1
        if (depth == 3 && keys[1] == "jobs:" && keys[3] == "uses:" &&
            use == "carina-rs/provider-release/.github/workflows/build.yml@" ref)
            build[FILENAME] = 1
    }
    END {
        if (count != 1 || invalid) {
            print "::error::All provider-release uses must share one full 40-hex commit SHA; found " count + 0 " distinct ref(s):"
            for (ref in refs) printf "  @%s:\n%s", ref, refs[ref]
            failed = 1
        }
        required[ci] = required[release] = 1
        for (file in required) {
            if (!(file in build)) {
                print "::error::" file " must contain a uses: carina-rs/provider-release/.github/workflows/build.yml@<sha> job."
                failed = 1
            }
        }
        exit failed
    }
' "${workflows[@]}"

toolchain="$repo_root/.github/wasm-toolchain"
[[ -f "$toolchain" ]] || fail "Missing $toolchain; expected a single exact Rust version (X.Y.Z)."
if ! awk 'END { exit !(NR == 1 && $0 ~ /^[0-9]+\.[0-9]+\.[0-9]+$/) }' "$toolchain"; then
    fail "$toolchain must contain a single exact Rust version (X.Y.Z)."
fi

printf 'OK: provider-release uses share one full 40-hex commit SHA, CI and release use build.yml, and wasm-toolchain is a single exact X.Y.Z version.\n'
