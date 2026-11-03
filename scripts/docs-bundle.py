#!/usr/bin/env python3
"""Build the documentation archive published with a release."""

import argparse
import gzip
import io
import json
import os
import re
import sys
import tarfile
import tempfile
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
GENERATED_DOCS = ROOT / "generated-docs" / "github"
INDEX_SOURCE = ROOT / "docs" / "registry" / "index.md"

SLUG_RE = re.compile(r"[a-z0-9][a-z0-9_]*\Z")


class BundleError(Exception):
    """A clear, user-facing build error."""


@dataclass(frozen=True)
class Page:
    archive_path: str
    content: bytes


def _validate_source_slug(slug, description):
    if not SLUG_RE.fullmatch(slug):
        raise BundleError(
            f"invalid {description} slug {slug!r}: expected [a-z0-9][a-z0-9_]*"
        )
    if len(slug.encode("utf-8")) > 128:
        raise BundleError(f"{description} slug {slug!r} exceeds 128 bytes")


def _page_content(path):
    try:
        text = path.read_text(encoding="utf-8")
    except UnicodeDecodeError as error:
        raise BundleError(
            f"{path}: generated documentation is not UTF-8: {error}"
        ) from error

    lines = text.splitlines(keepends=True)
    if not lines or lines[0].rstrip("\r\n") != "---":
        raise BundleError(f"{path}: expected YAML frontmatter starting with '---'")

    closing = None
    for index, line in enumerate(lines[1:], start=1):
        if line.rstrip("\r\n") == "---":
            closing = index
            break
    if closing is None:
        raise BundleError(f"{path}: YAML frontmatter has no closing '---'")

    title_values = []
    for line in lines[1:closing]:
        match = re.fullmatch(r"title:\s*(.*?)\s*(?:\r?\n)?", line)
        if match:
            title_values.append(match.group(1))
    if len(title_values) != 1:
        raise BundleError(f"{path}: YAML frontmatter must contain exactly one title")

    raw_title = title_values[0]
    try:
        title = json.loads(raw_title)
    except json.JSONDecodeError as error:
        raise BundleError(f"{path}: invalid title in YAML frontmatter: {error}") from error
    if not isinstance(title, str) or not title or "\n" in title or "\r" in title:
        raise BundleError(f"{path}: YAML frontmatter title must be a non-empty string")

    body = "".join(lines[closing + 1 :]).lstrip("\r\n")
    return f"# {title}\n\n{body}".encode("utf-8")


def _collect_pages(source_root=GENERATED_DOCS):
    if not source_root.is_dir():
        raise BundleError(f"generated documentation directory does not exist: {source_root}")

    destinations = {}
    for source in sorted(source_root.rglob("*.md")):
        relative = source.relative_to(source_root)
        if len(relative.parts) != 2:
            raise BundleError(
                f"{source}: expected generated-docs/github/<service>/<name>.md"
            )

        service = relative.parts[0]
        stem = source.stem
        is_data_source = stem.endswith("_data_source")
        name = stem[: -len("_data_source")] if is_data_source else stem
        _validate_source_slug(service, "service")
        _validate_source_slug(name, "document name")

        slug = f"{service}_{name}"
        _validate_source_slug(slug, "bundle")
        section = "data-sources" if is_data_source else "resources"
        archive_path = f"{section}/{slug}.md"
        if archive_path in destinations:
            previous = destinations[archive_path][0]
            raise BundleError(
                f"two sources map to {archive_path}: {previous} and {source}"
            )

        content = _page_content(source)
        destinations[archive_path] = (source, Page(archive_path, content))

    if not destinations:
        raise BundleError(
            "no resource or data-source documentation pages found in "
            f"{source_root}"
        )

    return [value[1] for value in destinations.values()]


def _build_index(index_source=INDEX_SOURCE):
    try:
        introduction = index_source.read_text(encoding="utf-8")
    except FileNotFoundError as error:
        raise BundleError(
            f"provider documentation index does not exist: {index_source}"
        ) from error
    except UnicodeDecodeError as error:
        raise BundleError(
            f"{index_source}: provider index is not UTF-8: {error}"
        ) from error

    return (introduction.rstrip() + "\n").encode("utf-8")


def _tar_info(name, *, size=0, directory=False):
    info = tarfile.TarInfo(name)
    info.type = tarfile.DIRTYPE if directory else tarfile.REGTYPE
    info.size = 0 if directory else size
    info.mode = 0o755 if directory else 0o644
    info.mtime = 0
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    return info


def build_bundle(output, source_root=GENERATED_DOCS, index_source=INDEX_SOURCE):
    pages = _collect_pages(source_root)
    index = _build_index(index_source)

    entries = [("index.md", False, index)]
    for directory in ("resources", "data-sources"):
        if any(page.archive_path.startswith(directory + "/") for page in pages):
            entries.append((directory, True, b""))
    entries.extend((page.archive_path, False, page.content) for page in pages)
    entries.sort(key=lambda entry: entry[0])

    output = Path(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="wb", dir=output.parent, prefix=f".{output.name}.", delete=False
        ) as raw:
            temporary = Path(raw.name)
            with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
                with tarfile.open(
                    mode="w", fileobj=compressed, format=tarfile.PAX_FORMAT
                ) as archive:
                    for name, directory, content in entries:
                        info = _tar_info(
                            name, size=len(content), directory=directory
                        )
                        archive.addfile(
                            info, None if directory else io.BytesIO(content)
                        )
        os.replace(temporary, output)
        temporary = None
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)

    print(f"built {output} ({len(pages) + 1} files)")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    build_parser = subparsers.add_parser("build", help="build the docs bundle")
    build_parser.add_argument("output", type=Path)
    args = parser.parse_args(argv)

    try:
        build_bundle(args.output)
        return 0
    except (BundleError, OSError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
