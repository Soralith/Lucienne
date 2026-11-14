"""Download the remote images referenced by the converted templates/data.

Run after exporting (and converting) so every Unsplash URL the UI references
exists under ui/static/dashboard/remote/. Re-running skips files already saved.

    python3 Lucienne/scripts/fetch_assets.py
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from media_map import REMOTE_DIR, local_name, urls

ROOT = Path(__file__).resolve().parents[1]


def gather() -> set[str]:
    found: set[str] = set()
    sources = (
        list((ROOT / "data").glob("*.json"))
        + list((ROOT / "ui" / "templates").rglob("*.html"))
        + list((ROOT / "reference").glob("*.html"))
        # Rewritten copies no longer contain the URLs, so rescan the source
        # templates and product data they came from.
        + list((ROOT.parent / "dashboard" / "templates").rglob("*.html"))
        + [ROOT / ".." / "dashboard" / "static" / "dashboard" / "js" / "products.js"]
    )
    for path in sources:
        if path.exists():
            found |= urls(path.read_text(encoding="utf-8"))
    return found


def main() -> int:
    REMOTE_DIR.mkdir(parents=True, exist_ok=True)
    missing = 0
    pending = gather()
    print(f"{len(pending)} remote images referenced")
    for url in sorted(pending):
        target = REMOTE_DIR / local_name(url)
        if target.exists() and target.stat().st_size > 0:
            continue
        result = subprocess.run(
            ["curl", "-fsSL", "--retry", "2", url, "-o", str(target)],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            missing += 1
            target.unlink(missing_ok=True)
            print(f"failed: {url}: {result.stderr.strip()[:120]}")
        else:
            print(f"saved {target.name} ({target.stat().st_size // 1024} KB)")
    print(f"done, {missing} failed")
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
