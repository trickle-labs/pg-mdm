#!/usr/bin/env python3
from hashlib import sha256
import json
from pathlib import Path
import sys

if len(sys.argv) != 2:
    sys.exit("usage: package_manifest.py PACKAGE_DIRECTORY")

package = Path(sys.argv[1]).resolve()
if not package.is_dir():
    sys.exit(f"package directory does not exist: {package}")

files = []
for path in sorted(item for item in package.rglob("*") if item.is_file()):
    content = path.read_bytes()
    files.append(
        {
            "path": path.relative_to(package).as_posix(),
            "bytes": len(content),
            "sha256": sha256(content).hexdigest(),
        }
    )

print(json.dumps({"package": package.name, "files": files}, indent=2))
