#!/usr/bin/env bash
# Exercise the provider-release guard with local workflow fixtures only.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
checker="$repo_root/scripts/check-provider-release-pin.sh"
fixture_dir=$(mktemp -d "${TMPDIR:-/tmp}/carina-provider-release.XXXXXX")
trap 'rm -rf "$fixture_dir"' EXIT
workflows_dir="$fixture_dir/.github/workflows"
sha=0123456789abcdef0123456789abcdef01234567
other_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa

write_fixture() {
    rm -rf "$fixture_dir/.github"
    mkdir -p "$workflows_dir"
    printf '1.95.0\n' > "$fixture_dir/.github/wasm-toolchain"
    for workflow in ci release; do
        printf 'jobs:\n  build:\n    uses: carina-rs/provider-release/.github/workflows/build.yml@%s\n' "$sha" > "$workflows_dir/$workflow.yml"
    done
}

failures=0
valid_fixtures=0
regressions=0

assert_accepted() {
    local output
    valid_fixtures=$((valid_fixtures + 1))
    if ! output=$(bash "$checker" "${2:-$workflows_dir}" 2>&1); then
        printf 'FAIL: guard rejected %s:\n%s\n' "$1" "$output" >&2
        failures=$((failures + 1))
    else
        printf 'PASS: accepted %s\n' "$1"
    fi
}

assert_rejected() {
    local output expected
    regressions=$((regressions + 1))
    if output=$(bash "$checker" "$workflows_dir" 2>&1); then
        printf 'FAIL: guard accepted %s\n' "$1" >&2
        failures=$((failures + 1))
    else
        for expected in '::error::' "${@:2}"; do
            if [[ "$output" != *"$expected"* ]]; then
                printf 'FAIL: unexpected diagnostic for %s:\n%s\n' "$1" "$output" >&2
                failures=$((failures + 1))
                return
            fi
        done
        printf 'PASS: rejected %s\n' "$1"
    fi
}

write_fixture
assert_accepted 'matching SHAs'
printf '# uses: carina-rs/provider-release/publish@main\n  # - uses: carina-rs/provider-release/yank@main\n' >> "$workflows_dir/ci.yml"
printf 'jobs:\n  extra:\n    steps:\n      - uses: "carina-rs/provider-release/publish@%s" # pinned\n      - uses: '\''carina-rs/provider-release/yank@%s'\''\n' "$sha" "$sha" > "$workflows_dir/extra.yaml"
assert_accepted 'comments, quoted refs, and .yaml workflows'

printf '      - uses: carina-rs/provider-release/github-release@%s\n' "$other_sha" >> "$workflows_dir/extra.yaml"
assert_rejected 'two different SHAs' "@$sha:" 'ci.yml:3' 'release.yml:3' "@$other_sha:" 'extra.yaml:6'

write_fixture
printf 'jobs:\n  extra:\n    steps:\n      - uses: carina-rs/provider-release/publish@main\n' > "$workflows_dir/extra.yaml"
assert_rejected 'a non-SHA ref' '@main:' 'extra.yaml:4'

write_fixture
printf 'jobs:\n  release:\n    steps:\n      - uses: carina-rs/provider-release/publish@%s\n#    uses: carina-rs/provider-release/.github/workflows/build.yml@%s\n' "$sha" "$sha" > "$workflows_dir/release.yml"
assert_rejected 'release.yml without a build.yml job' 'release.yml must contain' 'build.yml@<sha> job'

write_fixture
printf 'jobs:\n  ci:\n    steps:\n      - name: Not a reusable workflow job\n        uses: carina-rs/provider-release/.github/workflows/build.yml@%s\n' "$sha" > "$workflows_dir/ci.yml"
assert_rejected 'ci.yml with build.yml only in a step' 'ci.yml must contain' 'build.yml@<sha> job'

write_fixture
printf 'stable\n' > "$fixture_dir/.github/wasm-toolchain"
assert_rejected 'a bad wasm-toolchain (stable)' 'wasm-toolchain must contain a single exact Rust version (X.Y.Z)'
printf '1.95.0\n\n' > "$fixture_dir/.github/wasm-toolchain"
assert_rejected 'a multiline wasm-toolchain' 'wasm-toolchain must contain a single exact Rust version (X.Y.Z)'
rm "$fixture_dir/.github/wasm-toolchain"
assert_rejected 'a missing wasm-toolchain' 'Missing' 'wasm-toolchain'

assert_accepted 'the real repository' "$repo_root/.github/workflows"

if [[ "$failures" -gt 0 ]]; then
    printf 'FAIL: provider-release pin guard failed %s fixture(s).\n' "$failures" >&2
    exit 1
fi

printf 'OK: provider-release pin guard accepted all %s valid fixtures and rejected all %s regressions.\n' "$valid_fixtures" "$regressions"
