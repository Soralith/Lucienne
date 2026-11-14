"""Export the Kaimono demo data to JSON for the Tauri build.

Run from the repository root with the project's virtualenv and a scratch
SQLite database (the shipped settings point at a retired Supabase tenant):

    DATABASE_URL=sqlite:////tmp/lucienne.db main/bin/python Lucienne/scripts/export_seed.py

It seeds the DB with the static catalog + demo community, then writes
`Lucienne/data/seed.json` and reference renders in `Lucienne/reference/`.
"""

import datetime
import decimal
import json
import os
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from media_map import rewrite  # noqa: E402

import django

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
os.environ.setdefault("DJANGO_SETTINGS_MODULE", "kaimono.settings")
django.setup()

from django.test import Client  # noqa: E402
from django.test.utils import setup_test_environment  # noqa: E402
from django.forms import BaseForm  # noqa: E402
from accounts.models import User  # noqa: E402
from dashboard.models import (  # noqa: E402
    ShopProduct,
    LibraryGame,
    UserWishlistItem,
    GameSubmission,
    GameReview,
    CommunityStory,
    CommunityChannel,
    CommunityGame,
    CommunityPost,
    CommunityComment,
    CommunityMember,
)


def load_products():
    text = (ROOT / "dashboard/static/dashboard/js/products.js").read_text(encoding="utf-8")
    raw = re.search(r"window\.KAIMONO_PRODUCTS\s*=\s*(\[.*?\]);", text, re.DOTALL).group(1)
    raw = re.sub(r"(?<=[{,])\s*([A-Za-z_$][\w$]*)\s*:", r'"\1":', raw)
    raw = re.sub(r",\s*([}\]])", r"\1", raw)
    return json.loads(raw)


def plain(value):
    if isinstance(value, (datetime.datetime, datetime.date)):
        return value.isoformat()
    if isinstance(value, decimal.Decimal):
        return float(value)
    return value


def model_dict(obj, extra=()):
    data = {}
    for field in obj._meta.fields:
        value = getattr(obj, field.attname if field.is_relation else field.name)
        data[field.name] = plain(value)
    for key in extra:
        if hasattr(obj, key):
            data[key] = plain(getattr(obj, key))
    for key, value in vars(obj).items():
        if not key.startswith("_") and key not in data:
            data[key] = plain(value)
    if hasattr(obj, "get_role_display") and hasattr(obj, "is_authenticated"):
        # Django template conveniences the converted templates rely on.
        data["pk"] = obj.pk
        data["is_authenticated"] = bool(obj.is_authenticated)
        data["get_role_display"] = obj.get_role_display()
    return data


def seed_shop(products):
    ShopProduct.objects.all().delete()
    for p in products:
        ShopProduct.objects.create(
            name=p["name"],
            category=p.get("category", "games"),
            price=p.get("price"),
            original_price=p.get("originalPrice"),
            image=p.get("image", ""),
            data=p,
        )


def seed_user(products):
    user, _ = User.objects.get_or_create(
        username="kaichan",
        defaults={"email": "kaichan@kaimono.id"},
    )
    user.email = "kaichan@kaimono.id"
    user.display_name = "Kaichan"
    user.role = "developer"
    user.is_superuser = True
    user.is_staff = True
    user.bio = "Collector of JRPGs, mechanical keyboards and limited figures."
    user.twitter = "@kaichan"
    user.instagram = "@kaichan.plays"
    user.youtube = "@kaichanplays"
    user.theme = "silver"
    user.age = 24
    user.set_password("password123")
    user.save()

    LibraryGame.objects.filter(user__isnull=True).update(user=user)
    LibraryGame.objects.filter(user=user).delete()
    picks = [p for p in products if p.get("category") == "games"][:6]
    statuses = ["Now Playing", "Installed", "Installed", "Not Installed", "Installed", "Installed"]
    for index, p in enumerate(picks):
        LibraryGame.objects.create(
            user=user,
            title=p["name"],
            studio=p.get("brand", "Kaimono Studio"),
            image_url=p.get("image", ""),
            meta=p.get("type", "Game"),
            status=statuses[index % len(statuses)],
            favorite=index in (0, 2),
        )

    UserWishlistItem.objects.filter(user=user).delete()
    wish = [p for p in products if p.get("category") == "games"][2:6] + [
        p for p in products if p.get("category") != "games"
    ][:2]
    for p in wish:
        product = ShopProduct.objects.get(name=p["name"])
        UserWishlistItem.objects.create(user=user, product=product)
    return user


def seed_extra(user):
    if not GameSubmission.objects.exists():
        GameSubmission.objects.create(
            developer=user,
            name="Neon Drifters",
            category="games",
            type="Roguelike",
            brand="Studio Halcyon",
            price=19.99,
            original_price=24.99,
            description="A neon-soaked roguelike about drifting through collapsing cities.",
            tags="roguelike, pixel art",
            trailer_url="https://example.com/trailer",
            stock="In Stock",
            badges="New",
            developer_name="Studio Halcyon",
            publisher="Studio Halcyon",
            release_date="2026-10-30",
            status="pending",
        )
        GameSubmission.objects.create(
            developer=user,
            name="Paper Lanterns",
            category="games",
            type="Puzzle",
            brand="Moth & Moon",
            price=12.5,
            description="A quiet puzzle game about lighting lanterns for absent friends.",
            tags="cozy, puzzle",
            stock="In Stock",
            developer_name="Moth & Moon",
            publisher="Moth & Moon",
            release_date="2026-08-11",
            status="accepted",
        )


def dump():
    products = load_products()
    seed_shop(products)
    user = seed_user(products)
    seed_extra(user)

    data = {
        "products": [model_dict(p) for p in ShopProduct.objects.order_by("id")],
        "users": [model_dict(u) for u in User.objects.all()],
        "library": [
            model_dict(g) for g in LibraryGame.objects.filter(user=user).order_by("created_at")
        ],
        "wishlist": [
            {
                "wishlist_id": w.id,
                "product_id": w.product_id,
                "created_at": plain(w.created_at),
                "product": model_dict(w.product),
            }
            for w in UserWishlistItem.objects.filter(user=user).select_related("product")
        ],
        "posts": [],
        "channels": [model_dict(c) for c in CommunityChannel.objects.all()],
        "stories": [model_dict(s) for s in CommunityStory.objects.all()],
        "games": [model_dict(g) for g in CommunityGame.objects.all()],
        "members": [model_dict(m) for m in CommunityMember.objects.all()],
        "submissions": [model_dict(s) for s in GameSubmission.objects.all()],
        "reviews": [model_dict(r) for r in GameReview.objects.all()],
        "comments": [model_dict(c) for c in CommunityComment.objects.all()],
    }

    for post in CommunityPost.objects.prefetch_related(
        "images", "tags", "reaction_types", "poll__options"
    ):
        item = model_dict(post)
        item["images"] = [model_dict(i) for i in post.images.all()]
        item["tags"] = [model_dict(t) for t in post.tags.all()]
        item["reactions"] = [model_dict(r) for r in post.reaction_types.all()]
        item["comments"] = [
            model_dict(c)
            for c in CommunityComment.objects.filter(post=post).order_by("created_at")
        ]
        item["is_owner"] = post.author_name == (user.display_name or user.username)
        if hasattr(post, "poll"):
            poll = post.poll
            options = []
            total = poll.votes.count()
            for option in poll.options.all():
                count = option.votes.count()
                row = model_dict(option)
                row["percentage"] = round(count / total * 100) if total else 0
                row["is_selected"] = False
                options.append(row)
            item["poll_obj"] = model_dict(poll)
            item["poll_options"] = options
            item["poll_voted"] = False
            item["poll_selected_id"] = None
            item["poll_total"] = total
        else:
            item["poll_obj"] = None
            item["poll_options"] = []
            item["poll_voted"] = False
            item["poll_selected_id"] = None
            item["poll_total"] = 0
        data["posts"].append(item)

    out = OUT / "data"
    out.mkdir(exist_ok=True)
    (out / "seed.json").write_text(rewrite(json.dumps(data, indent=2)), encoding="utf-8")
    print(f"wrote {out / 'seed.json'} ({len(data['products'])} products, {len(data['posts'])} posts)")

    # Reference renders for visual/HTML diffing against the Rust renderer.
    reference = OUT / "reference"
    reference.mkdir(exist_ok=True)
    client = Client()
    client.force_login(user)
    pages = {
        "dashboard": "/dashboard/",
        "shop": "/shop/",
        "community": "/community/",
        "wishlist": "/wishlist/",
        "library": "/library/",
        "profile": "/profile/",
        "settings": "/settings/",
        "developer": "/settings/developer/",
        "admin": "/settings/admin/",
    }
    for p in data["products"]:
        pages[f"detail-{p['id']}"] = f"/shop/{p['id']}/"
    setup_test_environment()
    contexts = {}
    for name, url in pages.items():
        response = client.get(url)
        (reference / f"{name}.html").write_text(response.content.decode("utf-8"), encoding="utf-8")
        context = {}
        layers = getattr(response, "context", None) or []
        for layer in layers:
            if hasattr(layer, "flatten"):
                layer = layer.flatten()
            if not hasattr(layer, "items"):
                continue
            for key, value in layer.items():
                context[str(key)] = serializable(value)
        try:
            json.dumps(json_safe(context))
        except TypeError as exc:
            print(f"context {name} not serialisable: {exc}")
        contexts[name] = context
        print(f"{name}: {response.status_code} {len(response.content)} bytes, {len(context)} keys")

    # The community view hands model instances to the template, so reverse
    # relations and the channel FK never reach the serializer. Merge them in
    # from the seeded post dicts (same rows, prefetched relations).
    # Shop products default to visible when data has no explicit flag; the
    # template's default_if_none only applies to None.
    for product in contexts.get("admin", {}).get("products", []):
        if isinstance(product.get("data"), dict):
            product["data"].setdefault("active", True)

    if "community" in contexts:
        channels = {c["id"]: c for c in contexts["community"].get("channels", [])}
        seeded = {p["id"]: p for p in data["posts"]}
        for post in contexts["community"].get("posts", []):
            source = seeded.get(post.get("id"), {})
            for key in ("images", "tags", "reactions", "comments"):
                post[key] = source.get(key, [])
            if isinstance(post.get("channel"), int):
                post["channel"] = channels.get(post["channel"], post["channel"])

    # Same FK hydration for the admin dashboard's post list.
    if "admin" in contexts:
        channels = {c["id"]: c for c in contexts["admin"].get("channels", [])}
        for post in contexts["admin"].get("posts", []):
            if isinstance(post.get("channel"), int):
                post["channel"] = channels.get(post["channel"], post["channel"])

    (out / "contexts.json").write_text(
        rewrite(json.dumps(json_safe(contexts), indent=2)), encoding="utf-8"
    )
    print(f"wrote {out / 'contexts.json'}")


def json_safe(value):
    if isinstance(value, str):
        return str(value)
    if isinstance(value, dict):
        return {str(k): json_safe(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [json_safe(v) for v in value]
    return value


def serializable(value):
    # Beware of Django's lazy proxies: they pass isinstance checks against the
    # wrapped type but are not encodable by json, so coerce strings early.
    if isinstance(value, str):
        return str(value)
    if value is None or isinstance(value, (bool, int, float)):
        return value
    if isinstance(value, BaseForm):
        data = {"instance": serializable(value.instance)}
        # Some templates read fields the form does not own (e.g. the fixed role).
        for extra in ("role", "username", "email"):
            if extra not in value.fields and hasattr(value.instance, extra):
                data[extra] = {
                    "value": getattr(value.instance, extra),
                    "errors": [],
                }
        data["non_field_errors"] = [str(e) for e in value.non_field_errors()]
        for name, field in value.fields.items():
            data[name] = {
                # BoundField.value() also falls back to the model instance for
                # unbound model forms, which is what the Django templates see.
                "value": value[name].value(),
                "errors": [str(e) for e in value[name].errors] if value.is_bound else [],
            }
        return data
    if hasattr(value, "_meta"):
        return model_dict(value)
    if isinstance(value, dict):
        return {str(k): serializable(v) for k, v in value.items()}
    if isinstance(value, (list, tuple, set)):
        return [serializable(v) for v in value]
    if isinstance(value, (datetime.datetime, datetime.date, decimal.Decimal)):
        return plain(value)
    if hasattr(value, "all"):
        try:
            return [serializable(v) for v in value.all()]
        except TypeError:
            return None
    if callable(value):
        return None
    return str(value)


if __name__ == "__main__":
    dump()
