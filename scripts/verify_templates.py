"""Render the converted Jinja templates against the exported contexts.

This mirrors what the Rust server does at runtime so template problems show up
before a Tauri build. Each page is rendered into Lucienne/rendered/ and compared
with the Django reference output in Lucienne/reference/.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

from jinja2 import ChainableUndefined, Environment, FileSystemLoader

ROOT = Path(__file__).resolve().parents[1]

PAGE_TEMPLATES = {
    "detail": "game_detail",
    "developer": "developer_panel",
    "admin": "admin_panel",
}

# Django filters used by the converted templates.
def ddefault(value, arg=None):
    return arg if not value else value


def ddefault_if_none(value, arg=None):
    return arg if value is None else value


def truncatechars(value, count):
    text = str(value)
    count = int(count)
    return text if len(text) <= count else text[:count] + "…"


def truncatewords(value, count):
    words = str(value).split()
    count = int(count)
    return " ".join(words[:count]) + (" …" if len(words) > count else "")


def floatformat(value, decimals=None):
    try:
        number = float(value)
    except (TypeError, ValueError):
        return ""
    digits = 1 if decimals in (None, "") else int(decimals)
    return f"{number:.{digits}f}"


def yesno(value, arg=None):
    choices = (arg or "yes,no,maybe").split(",")
    if value:
        return choices[0]
    return choices[1] if len(choices) > 1 else ""


def pluralize(value, arg=None):
    try:
        number = float(value)
    except (TypeError, ValueError):
        number = 0
    if not arg:
        return "" if number == 1 else "s"
    choices = arg.split(",")
    if number == 1:
        return choices[0]
    return choices[1] if len(choices) > 1 else choices[0]


def escapejs(value):
    return json.dumps(str(value), ensure_ascii=False)[1:-1]


DJANGO_MONTHS = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
]


def ddate(value, arg=None):
    """Subset of Django's date filter format codes used by the templates."""
    text = str(value)
    match = re.match(r"(\d{4})-(\d{2})-(\d{2})", text)
    if not match or not arg:
        return text[:10]
    year, month, day = (int(part) for part in match.groups())
    tokens = {
        "Y": f"{year:04d}",
        "m": f"{month:02d}",
        "d": f"{day:02d}",
        "j": str(day),
        "n": str(month),
        "F": DJANGO_MONTHS[month - 1],
        "M": DJANGO_MONTHS[month - 1][:3],
        "y": f"{year % 100:02d}",
    }
    return "".join(tokens.get(ch, ch) for ch in arg)


def build_env() -> Environment:
    env = Environment(
        loader=FileSystemLoader(str(ROOT / "ui" / "templates")),
        autoescape=True,
        undefined=ChainableUndefined,
    )
    env.filters.update(
        ddefault=ddefault,
        ddefault_if_none=ddefault_if_none,
        truncatechars=truncatechars,
        truncatewords=truncatewords,
        floatformat=floatformat,
        yesno=yesno,
        pluralize=pluralize,
        escapejs=escapejs,
        ddate=ddate,
    )
    return env


def hydrate(context: dict, products: dict[int, dict]) -> dict:
    """Replace product ids with product objects, like the Rust context loader."""
    for key in ("wishlist_items", "library_items", "reviews", "submissions", "posts"):
        items = context.get(key)
        if not isinstance(items, list):
            continue
        for item in items:
            if not isinstance(item, dict):
                continue
            pid = item.get("product")
            if isinstance(pid, int):
                item["product"] = products.get(pid, {"id": pid, "data": {}})
    for item in context.get("wishlist_items") or []:
        product = item.get("product") if isinstance(item, dict) else None
        if isinstance(product, dict):
            product.setdefault("data", {})
    user = context.get("user")
    if isinstance(user, dict):
        context["user"] = user
        context["request"] = {"path": "/", "user": user, "GET": {}}
    # Django template-side objects have no JSON equivalent.
    context["messages"] = []
    context["perms"] = {}
    return context


def template_name(page: str) -> str:
    base = "detail" if page.startswith("detail-") else page
    return f"dashboard/pages/{PAGE_TEMPLATES.get(base, base)}.html"


def main() -> int:
    contexts = json.loads((ROOT / "data" / "contexts.json").read_text())
    seed = json.loads((ROOT / "data" / "seed.json").read_text())
    products = {product["id"]: product for product in seed.get("products", [])}
    env = build_env()
    out_dir = ROOT / "rendered"
    out_dir.mkdir(exist_ok=True)

    failures = 0
    for page, context in contexts.items():
        try:
            html = env.get_template(template_name(page)).render(**hydrate(context, products))
        except Exception as exc:  # noqa: BLE001 - report every page failure
            failures += 1
            print(f"FAIL {page}: {type(exc).__name__}: {exc}")
            continue
        (out_dir / f"{page}.html").write_text(html)
        reference = ROOT / "reference" / (
            "details.html" if page.startswith("detail-") else f"{page}.html"
        )
        if reference.exists():
            text = reference.read_text()
            delta = len(html) - len(text)
            print(f"ok   {page}: {len(html)} chars (reference {len(text)}, {delta:+d})")
        else:
            print(f"ok   {page}: {len(html)} chars")

    print(f"\n{len(contexts) - failures}/{len(contexts)} pages rendered")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
