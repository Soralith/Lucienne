"""Convert the Django templates to Jinja (minijinja) templates for the Tauri UI.

The markup is copied verbatim; only template tags/filters change. Landing,
login, account setup and the AI companion are dropped from this build.

    python3 Lucienne/scripts/convert_templates.py
"""

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from media_map import rewrite  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "dashboard" / "templates" / "dashboard"
DST = Path(__file__).resolve().parents[1] / "ui" / "templates" / "dashboard"

SKIP_PAGES = {"landing.html", "login.html", "accsetup.html", "ai_companion.html"}

URLS = {
    "landing": "/",
    "login": "/login/",
    "register": "/register/",
    "logout": "/logout/",
    "accsetup": "/accsetup/",
    "profile": "/profile/",
    "dashboard": "/dashboard/",
    "shop": "/shop/",
    "game_detail": "/shop/{0}/",
    "community": "/community/",
    "wishlist": "/wishlist/",
    "library": "/library/",
    "settings": "/settings/",
    "ai_companion": "/settings/ai-companion/",
    "delete_account": "/settings/delete-account/",
    "admin_panel": "/settings/admin/",
    "developer_panel": "/settings/developer/",
    "developer_panel_submit": "/settings/developer/submit/",
    "community_vote_poll": "/api/community/vote/",
    "community_create_post": "/api/community/post/",
    "community_edit_post": "/api/community/post/edit/",
    "community_delete_post": "/api/community/post/delete/",
    "community_add_comment": "/api/community/comment/",
    "community_get_comments": "/api/community/comments/",
    "community_share_post": "/api/community/share/",
    "community_toggle_follow_game": "/api/community/games/follow/",
    "checkout_complete": "/api/shop/checkout/",
    "api_check_ownership": "/api/shop/check-ownership/",
    "api_wishlist_toggle": "/api/wishlist/toggle/",
    "api_wishlist_list": "/api/wishlist/list/",
    "api_game_reviews": "/api/reviews/",
    "api_game_review_submit": "/api/reviews/submit/",
    "ai_companion_chat": "/api/ai/chat/",
    "ai_companion_clear": "/api/ai/clear/",
}

FILTERS_WITH_ARG = {
    "default_if_none": "ddefault_if_none",
    "default": "ddefault",
    "truncatechars": "truncatechars",
    "truncatewords": "truncatewords",
    "floatformat": "floatformat",
    "yesno": "yesno",
    "pluralize": "pluralize",
    "date": "ddate",
}


def convert_urls(text):
    def repl(match):
        body = match.group(1).strip()
        parts = body.split()
        if not parts:
            return ""
        name = parts[0].strip("'\"")
        args = parts[1:]
        path = URLS.get(name)
        if path is None:
            return "/"
        if args and "{0}" in path:
            return path.format("{{ " + " ".join(args) + " }}")
        return path

    return re.sub(r"\{%\s*url\s+(.+?)\s*%\}", repl, text)


def convert_filters(text):
    # Django filters appear inside {{ }} and {% if %} alike, so rewrite them
    # everywhere. Arguments first, then the bare no-argument forms.
    for django_name, jinja_name in FILTERS_WITH_ARG.items():
        pattern = re.compile(r"\|" + django_name + r":(\"[^\"]*\"|'[^']*'|[^|\s}%]+)")
        text = pattern.sub(lambda m: f"|{jinja_name}({m.group(1)})", text)
    text = re.sub(
        r"\|(floatformat|pluralize|escapejs|ddate)(?![\w(])",
        lambda m: f"|{m.group(1)}()",
        text,
    )
    return text


def convert(text):
    # Bundle the runtime assets so the desktop app works without a network.
    # Tailwind is precompiled: drop the CDN script and load the built stylesheet
    # last in <head>, after the page's own <style> block.
    text = re.sub(
        r'\s*<script src="https://cdn\.tailwindcss\.com[^"]*"></script>', "", text
    )
    text = text.replace(
        "</head>",
        '  <link rel="stylesheet" href="/static/dashboard/css/tailwind.css">\n</head>',
    )
    text = text.replace("https://unpkg.com/lucide@latest", "/static/dashboard/vendor/lucide.js")
    text = re.sub(
        r"https://fonts\.googleapis\.com/css2[^\"]*",
        "/static/dashboard/vendor/jakarta.css",
        text,
    )
    text = re.sub(r"\{%\s*load[^%]*%\}", "", text)
    text = re.sub(
        r"\{%\s*csrf_token\s*%\}",
        '<input type="hidden" name="csrfmiddlewaretoken" value="{{ csrf_token }}">',
        text,
    )
    text = convert_urls(text)
    text = re.sub(
        r"\{%\s*static\s+['\"](.+?)['\"]\s+as\s+(\w+)\s*%\}",
        lambda m: '{% set ' + m.group(2) + ' = "/static/' + m.group(1) + '" %}',
        text,
    )
    text = re.sub(
        r"\{%\s*static\s+['\"](.+?)['\"]\s*%\}",
        lambda m: "/static/" + m.group(1),
        text,
    )
    text = re.sub(r"\{%\s*empty\s*%\}", "{% else %}", text)
    text = re.sub(r"\{%\s*with\s+([^%]+?)%\}", lambda m: "{% with " + re.sub(r"(\w+)=", r"\1 = ", m.group(1)) + "%}", text)
    text = text.replace("forloop.counter0", "loop.index0")
    text = text.replace("forloop.counter", "loop.index")
    text = text.replace("forloop.revcounter", "loop.revindex")
    text = text.replace("forloop.", "loop.")
    text = text.replace(".images.all", ".images").replace(".tags.all", ".tags")
    # Queryset.first -> list index (the surrounding {% if %} guards emptiness).
    text = text.replace(".images.first.url", ".images[0].url")
    text = convert_filters(text)
    # Django's if-tag coerces the floatformat result; Jinja does not compare int to str.
    text = text.replace(
        "loop.index <= new_release.rating|floatformat(0)",
        "loop.index <= new_release.rating",
    )
    text = rewrite(text)
    return text


def main():
    count = 0
    for path in sorted(SRC.rglob("*.html")):
        if path.parent.name == "pages" and path.name in SKIP_PAGES:
            continue
        if path.parent.name == "pages" and path.name == "ai_companion.html":
            continue
        target = DST / path.relative_to(SRC)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(convert(path.read_text(encoding="utf-8")), encoding="utf-8")
        count += 1
        print("converted", path.relative_to(SRC))
    print(f"{count} templates -> {DST}")


if __name__ == "__main__":
    main()
