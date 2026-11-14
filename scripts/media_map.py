"""Map remote Unsplash URLs to bundled local files.

The templates and exported data are rewritten to point at
/static/dashboard/remote/<hash>.jpg so the desktop build renders identically
without a network connection. Lucienne/scripts/fetch_assets.py downloads the
matching files.
"""

from __future__ import annotations

import hashlib
import re
from pathlib import Path

URL_RE = re.compile(r"https://images\.unsplash\.com/[^\"'\s)\\]*")
ROOT = Path(__file__).resolve().parents[1]
REMOTE_DIR = ROOT / "ui" / "static" / "dashboard" / "remote"


def local_name(url: str) -> str:
    digest = hashlib.md5(url.encode("utf-8")).hexdigest()[:16]
    return f"{digest}.jpg"


def local_path(url: str) -> str:
    return f"/static/dashboard/remote/{local_name(url)}"


def rewrite(text: str) -> str:
    return URL_RE.sub(lambda match: local_path(match.group(0)), text)


def urls(text: str) -> set[str]:
    return set(URL_RE.findall(text))


__all__ = ["REMOTE_DIR", "local_name", "local_path", "rewrite", "urls"]
