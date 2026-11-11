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

write_build() {
    printf '%s:\n  build:\n    uses: carina-rs/provider-release/.github/workflows/%s.yml@%s\n    with:\n      provider: %s\n' \
        "${4:-jobs}" "${3:-build}" "${2:-$sha}" "${5:-carina-provider-fixture}" > "$workflows_dir/$1.yml"
}

write_fixture() {
    rm -rf "$fixture_dir/.github"
    mkdir -p "$workflows_dir"
    printf '1.95.0\n' > "$fixture_dir/.github/wasm-toolchain"
    printf '[toolchain]\nchannel = "1.95.0"\n' > "$fixture_dir/rust-toolchain.toml"
    for workflow in ci release; do
        write_build "$workflow" "${1:-$sha}"
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
    elif [[ "$output" != *'::error::'* || "${output#*::error::}" == *'::error::'* ]]; then
        printf 'FAIL: expected exactly one error for %s:\n%s\n' "$1" "$output" >&2
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
assert_accepted 'matching SHAs and Rust toolchain versions'
write_build release "$sha" build jobs "'carina-provider-fixture' # pinned"
printf '# uses: carina-rs/provider-release/publish@main\n  # - uses: carina-rs/provider-release/yank@main\n' >> "$workflows_dir/ci.yml"
printf 'jobs:\n  extra:\n    steps:\n      - uses: "carina-rs/provider-release/publish@%s" # pinned\n      - uses: '\''carina-rs/provider-release/yank@%s'\''\n' "$sha" "$sha" > "$workflows_dir/extra.yaml"
assert_accepted 'comments, quoted refs, and .yaml workflows'

printf '      - uses: carina-rs/provider-release/github-release@%s\n' "$other_sha" >> "$workflows_dir/extra.yaml"
assert_rejected 'two different SHAs' "@$sha:" 'ci.yml:3' 'release.yml:3' "@$other_sha:" 'extra.yaml:6'

write_fixture main
assert_rejected 'all uses consistently on @main' '@main:' 'ci.yml:3' 'release.yml:3'
write_fixture AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
assert_rejected 'a consistent 40-character uppercase-hex ref' '40-character lowercase hex' '@AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA:'
write_fixture abcdef
assert_rejected 'a consistent short lowercase-hex ref' '@abcdef:'

write_fixture
for workflow in ci release; do
    sed 's/carina-rs\/provider-release/Carina-rs\/Provider-Release/' "$workflows_dir/$workflow.yml" > "$fixture_dir/mixed.yml"
    mv "$fixture_dir/mixed.yml" "$workflows_dir/$workflow.yml"
done
assert_accepted 'mixed-case owner and repository in both build jobs'
write_fixture
printf 'jobs:\n  extra:\n    steps:\n      - uses: Carina-rs/provider-release/publish@main\n' > "$workflows_dir/extra.yaml"
assert_rejected 'a mixed-case owner with @main in an extra workflow' '@main:' 'extra.yaml:4'

write_fixture
printf 'jobs:\n  extra:\n    steps:\n      - uses: carina-rs/provider-release@%s\n' "$other_sha" > "$workflows_dir/extra.yaml"
assert_rejected 'a root reference with a different SHA' "@$other_sha:" 'extra.yaml:4'

write_fixture
write_build release "$sha" other
assert_rejected 'release.yml using other.yml instead of build.yml' 'release.yml must contain' 'build.yml@<sha> job'

write_fixture
write_build release "$sha" build other
assert_rejected 'a depth-3 build.yml use outside jobs' 'release.yml must contain' 'build.yml@<sha> job'

write_fixture
printf 'jobs:\n  release:\n    steps:\n      - uses: carina-rs/provider-release/publish@%s\n#    uses: carina-rs/provider-release/.github/workflows/build.yml@%s\n' "$sha" "$sha" > "$workflows_dir/release.yml"
assert_rejected 'release.yml without a build.yml job' 'release.yml must contain' 'build.yml@<sha> job'

write_fixture
printf 'jobs:\n  ci:\n    steps:\n      - name: Not a reusable workflow job\n        uses: carina-rs/provider-release/.github/workflows/build.yml@%s\n        with:\n          provider: carina-provider-fixture\n' "$sha" > "$workflows_dir/ci.yml"
assert_rejected 'ci.yml with build.yml only in a step' 'ci.yml must contain' 'build.yml@<sha> job'

write_fixture
write_build release "$sha" build jobs carina-provider-other
assert_rejected 'different provider inputs' 'identical with.provider values' 'ci.yml:5: carina-provider-fixture' 'release.yml:5: carina-provider-other'
write_build release
sed '/provider:/d' "$workflows_dir/release.yml" > "$fixture_dir/missing.yml"
mv "$fixture_dir/missing.yml" "$workflows_dir/release.yml"
assert_rejected 'a missing provider input in release' 'release.yml:3' 'must set with.provider'
printf '  unrelated:\n    with:\n      provider: carina-provider-fixture\n' >> "$workflows_dir/release.yml"
assert_rejected 'a provider supplied only by a later job' 'release.yml:3' 'must set with.provider'

write_fixture
printf 'jobs:\n  build:\n    uses: carina-rs/provider-release/.github/workflows/build.yml@%s\n    env:\n      provider: carina-provider-fixture\n' "$sha" > "$workflows_dir/release.yml"
assert_rejected 'a provider outside the with block' 'release.yml:3' 'must set with.provider'
printf 'jobs:\n  build:\n    uses: carina-rs/provider-release/.github/workflows/build.yml@%s\n    env:\n      with:\n        provider: carina-provider-fixture\n' "$sha" > "$workflows_dir/release.yml"
assert_rejected 'a provider in a nested with block' 'release.yml:3' 'must set with.provider'

write_fixture
printf 'stable\n' > "$fixture_dir/.github/wasm-toolchain"
assert_rejected 'a bad wasm-toolchain (stable)' 'wasm-toolchain must contain a single exact Rust version (X.Y.Z)'
printf '1.94.0\n1.95.0\n' > "$fixture_dir/.github/wasm-toolchain"
assert_rejected 'two valid wasm-toolchain versions' 'wasm-toolchain must contain a single exact Rust version (X.Y.Z)'
rm "$fixture_dir/.github/wasm-toolchain"
assert_rejected 'a missing wasm-toolchain' 'Missing' 'wasm-toolchain'

write_fixture
printf '1.94.0\n' > "$fixture_dir/.github/wasm-toolchain"
assert_rejected 'mismatched Rust toolchain versions' '.github/wasm-toolchain (1.94.0)' 'rust-toolchain.toml [toolchain].channel (1.95.0)'
printf '[toolchain]\nchannel = "1.94.0"\n' > "$fixture_dir/rust-toolchain.toml"
assert_accepted 'another matching Rust toolchain version'
rm "$fixture_dir/rust-toolchain.toml"
assert_rejected 'a missing rust-toolchain.toml' 'Missing' 'rust-toolchain.toml' '.github/wasm-toolchain'

write_fixture
printf '[toolchain]\nchannel = "stable"\n' > "$fixture_dir/rust-toolchain.toml"
assert_rejected 'a floating root toolchain' '.github/wasm-toolchain (1.95.0) must equal' 'rust-toolchain.toml [toolchain].channel (stable)'

assert_accepted 'the real repository' "$repo_root/.github/workflows"

if [[ "$failures" -gt 0 ]]; then
    printf 'FAIL: provider-release pin guard failed %s fixture(s).\n' "$failures" >&2
    exit 1
fi

printf 'OK: provider-release pin guard accepted all %s valid fixtures and rejected all %s regressions.\n' "$valid_fixtures" "$regressions"
