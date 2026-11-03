#!/usr/bin/env python3

import io
import os
import runpy
import tarfile
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "docs-bundle.py"
DOCS_BUNDLE = runpy.run_path(str(SCRIPT))
REMOVED_SERVICE_MANIFEST = Path("services").with_suffix(".json").name


def write_generated_page(path, title):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        f'---\ntitle: "{title}"\ndescription: "Fixture page"\n---\n\n'
        f"Documentation for {title}.\n",
        encoding="utf-8",
    )


def create_build_fixture(
    directory, *, reverse_creation=False, file_mode=0o644, mtime=0
):
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    source_root = directory / "generated-docs" / "github"
    pages = [
        ("alpha/standalone.md", "github.alpha.Standalone"),
        ("beta/shared.md", "github.beta.Shared"),
        ("beta/shared_data_source.md", "github.beta.Shared"),
        ("gamma/lookup_data_source.md", "github.gamma.Lookup"),
    ]
    index_source = directory / "index.md"

    if reverse_creation:
        index_source.write_text(
            "# GitHub fixture\n\nFixture introduction.\n\n \t\n", encoding="utf-8"
        )
        pages.reverse()

    page_paths = []
    for relative, title in pages:
        page_path = source_root / relative
        page_paths.append(page_path)
        write_generated_page(page_path, title)

    if not reverse_creation:
        index_source.write_text(
            "# GitHub fixture\n\nFixture introduction.\n\n \t\n", encoding="utf-8"
        )

    for fixture_file in [index_source, *page_paths]:
        os.chmod(fixture_file, file_mode)
        os.utime(fixture_file, (mtime, mtime))

    return source_root, index_source


class DocsBundleTests(unittest.TestCase):
    def build_fixture(self, directory, filename="docs.tar.gz", **fixture_options):
        directory = Path(directory)
        source_root, index_source = create_build_fixture(
            directory, **fixture_options
        )
        bundle = directory / filename
        with redirect_stdout(io.StringIO()):
            DOCS_BUNDLE["build_bundle"](bundle, source_root, index_source)
        return bundle

    def test_build_omits_removed_service_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            bundle = self.build_fixture(directory)
            with tarfile.open(bundle, "r:gz") as archive:
                self.assertNotIn(REMOVED_SERVICE_MANIFEST, archive.getnames())

    def test_build_is_byte_reproducible(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            first_root = directory / "first"
            second_root = directory / "second"
            first = self.build_fixture(
                first_root,
                file_mode=0o600,
                mtime=1_000_000_000,
            )
            second = self.build_fixture(
                second_root,
                reverse_creation=True,
                file_mode=0o644,
                mtime=1_500_000_000,
            )

            first_page = first_root / "generated-docs/github/alpha/standalone.md"
            second_page = second_root / "generated-docs/github/alpha/standalone.md"
            self.assertEqual(first_page.stat().st_mode & 0o777, 0o600)
            self.assertEqual(second_page.stat().st_mode & 0o777, 0o644)
            self.assertNotEqual(
                first_page.stat().st_mtime_ns,
                second_page.stat().st_mtime_ns,
            )

            self.assertEqual(first.read_bytes(), second.read_bytes())

    def test_build_replaces_frontmatter_with_title_heading_for_each_page_type(self):
        with tempfile.TemporaryDirectory() as directory:
            bundle = self.build_fixture(directory)

            with tarfile.open(bundle, "r:gz") as archive:
                cases = (
                    (
                        "resources/alpha_standalone.md",
                        "# github.alpha.Standalone\n\n",
                    ),
                    (
                        "data-sources/gamma_lookup.md",
                        "# github.gamma.Lookup\n\n",
                    ),
                )
                for archive_path, heading in cases:
                    with self.subTest(archive_path=archive_path):
                        member = archive.extractfile(archive_path)
                        self.assertIsNotNone(member)
                        content = member.read().decode("utf-8")
                        self.assertTrue(content.startswith(heading))
                        first_lines = "\n".join(content.splitlines()[:6])
                        self.assertNotIn("---", first_lines)
                        self.assertNotIn("title:", first_lines)

    def test_build_splits_pages_and_copies_only_provider_introduction_to_index(self):
        with tempfile.TemporaryDirectory() as directory:
            bundle = self.build_fixture(directory)
            with tarfile.open(bundle, "r:gz") as archive:
                self.assertEqual(
                    set(archive.getnames()),
                    {
                        "data-sources",
                        "data-sources/beta_shared.md",
                        "data-sources/gamma_lookup.md",
                        "index.md",
                        "resources",
                        "resources/alpha_standalone.md",
                        "resources/beta_shared.md",
                    },
                )
                member = archive.extractfile("index.md")
                self.assertIsNotNone(member)
                index = member.read().decode("utf-8")

        self.assertEqual(index, "# GitHub fixture\n\nFixture introduction.\n")

    def test_build_rejects_empty_generated_docs(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            source_root = directory / "generated-docs" / "github"
            source_root.mkdir(parents=True)
            index_source = directory / "index.md"
            index_source.write_text("# GitHub\n", encoding="utf-8")

            with self.assertRaisesRegex(
                DOCS_BUNDLE["BundleError"],
                "no resource or data-source documentation pages found",
            ):
                DOCS_BUNDLE["build_bundle"](
                    directory / "docs.tar.gz", source_root, index_source
                )


if __name__ == "__main__":
    unittest.main()
