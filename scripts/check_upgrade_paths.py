#!/usr/bin/env python3
from pathlib import Path
import re
import sys

root = Path(__file__).resolve().parent.parent
control = (root / "pg_mdm.control").read_text()
match = re.search(r"^default_version\s*=\s*'([^']+)'", control, re.MULTILINE)
if not match:
    sys.exit("pg_mdm.control has no default_version")

current = match.group(1)
archive = root / "sql" / "archive" / f"pg_mdm--{current}.sql"
if not archive.is_file():
    sys.exit(f"missing base archive: {archive.relative_to(root)}")

versions = {
    path.name.removeprefix("pg_mdm--").removesuffix(".sql")
    for path in (root / "sql" / "archive").glob("pg_mdm--*.sql")
}
edges = {}
for path in (root / "sql").glob("pg_mdm--*--*.sql"):
    source, target = path.stem.removeprefix("pg_mdm--").split("--", 1)
    edges.setdefault(source, set()).add(target)

for version in versions - {current}:
    pending = [version]
    seen = set()
    while pending:
        candidate = pending.pop()
        if candidate == current:
            break
        if candidate not in seen:
            seen.add(candidate)
            pending.extend(edges.get(candidate, ()))
    else:
        sys.exit(f"no upgrade path from {version} to {current}")

print(f"upgrade paths cover {len(versions)} archived version(s) through {current}")
