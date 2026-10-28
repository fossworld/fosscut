#!/usr/bin/env python3
"""Check that every ADR file is listed in docs/adr/README.md and is well formed (ADR-0001)."""

import re
import sys
from pathlib import Path

ADR_DIR = Path(__file__).resolve().parent.parent / "docs" / "adr"
FILE_RE = re.compile(r"^(\d{4})-[a-z0-9-]+\.md$")


def main() -> int:
    errors: list[str] = []
    index = (ADR_DIR / "README.md").read_text(encoding="utf-8")
    linked = set(re.findall(r"\]\((\d{4}-[a-z0-9-]+\.md)\)", index))

    files = sorted(p.name for p in ADR_DIR.glob("*.md") if FILE_RE.match(p.name))
    numbers = [int(FILE_RE.match(name).group(1)) for name in files]

    if len(numbers) != len(set(numbers)):
        errors.append("duplicate ADR numbers")
    expected = list(range(1, len(numbers) + 1))
    if numbers != expected:
        errors.append(f"ADR numbers must be consecutive from 0001, found {numbers}")

    for name in files:
        if name not in linked:
            errors.append(f"{name} is not listed in docs/adr/README.md")
        text = (ADR_DIR / name).read_text(encoding="utf-8")
        number = name[:4]
        if not text.startswith(f"# ADR-{number}: "):
            errors.append(f"{name} must start with '# ADR-{number}: <title>'")
        if not re.search(r"^- \*\*Status:\*\* ", text, re.MULTILINE):
            errors.append(f"{name} is missing a '- **Status:**' line")
        for section in ("## Context", "## Decision", "## Consequences"):
            if section not in text:
                errors.append(f"{name} is missing the '{section}' section")

    for name in sorted(linked):
        if not (ADR_DIR / name).exists():
            errors.append(f"docs/adr/README.md links to missing {name}")

    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"ADR index OK ({len(files)} records)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
