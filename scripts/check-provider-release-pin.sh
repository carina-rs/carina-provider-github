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
    function scalar(line) {
        sub(/^[^:]*:[[:space:]]*/, "", line)
        sub(/(^|[[:space:]])#.*/, "", line)
        sub(/[[:space:]]+$/, "", line)
        gsub(/^["\047]|["\047]$/, "", line)
        return line
    }
    FNR == 1 { depth = 0; current = "" }
    /^[[:space:]]*(#|$)/ { next }
    {
        # Track indentation so a step cannot stand in for a reusable workflow job.
        indent = match($0, /[^[:space:]]/)
        while (depth && indent <= indents[depth]) depth--
        indents[++depth] = indent
        keys[depth] = $1
        if (current && depth < use_depth) current = ""
    }
    /^[[:space:]]*(-[[:space:]]+)?uses:[[:space:]]*/ {
        use = scalar($0)
        if (tolower(use) !~ /^carina-rs\/provider-release(\/|@)/) next
        target = use
        sub(/@.*/, "", target)
        ref = use
        sub(/^[^@]*@/, "", ref)
        if (!(ref in refs)) count++
        refs[ref] = refs[ref] sprintf("    %s:%d\n", FILENAME, FNR)
        if (length(ref) != 40 || ref !~ /^[0-9a-f]+$/) invalid = 1
        if (depth == 3 && keys[1] == "jobs:" && $1 == "uses:" &&
            tolower(target) == "carina-rs/provider-release/.github/workflows/build.yml") {
            build[FILENAME] = 1
            current = FILENAME ":" FNR
            build_jobs[current] = FILENAME
            use_depth = depth
        }
    }
    current && depth == use_depth + 1 && keys[depth - 1] == "with:" && $1 == "provider:" {
        providers[current] = scalar($0)
        provider_lines[current] = FILENAME ":" FNR
    }
    END {
        if (count != 1 || invalid) {
            print "::error::All provider-release uses must share one full 40-character lowercase hex commit SHA; found " (count + 0) " distinct ref(s):"
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
        for (job in build_jobs) {
            if (!(build_jobs[job] in required)) continue
            if (!(job in providers) || providers[job] == "") {
                print "::error::" job " build.yml job must set with.provider."
                failed = 1
            }
            if (providers[job] != "") {
                if (!(providers[job] in values)) provider_count++
                values[providers[job]] = 1
                details = details sprintf("  %s: %s\n", provider_lines[job], providers[job])
            }
        }
        if (provider_count > 1) {
            print "::error::CI and release build.yml jobs must use identical with.provider values:"
            printf "%s", details
            failed = 1
        }
        exit (failed ? 1 : 0)
    }
' "${workflows[@]}"

toolchain="$repo_root/.github/wasm-toolchain"
[[ -f "$toolchain" ]] || fail "Missing $toolchain; expected a single exact Rust version (X.Y.Z)."
if ! awk 'END { exit !(NR == 1 && $0 ~ /^[0-9]+\.[0-9]+\.[0-9]+$/) }' "$toolchain"; then
    fail "$toolchain must contain a single exact Rust version (X.Y.Z)."
fi

printf 'OK: provider-release uses share one full lowercase 40-hex SHA, CI and release use build.yml with identical providers, and wasm-toolchain is a single exact X.Y.Z version.\n'
