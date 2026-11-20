//! Embedded HTTP server that renders the converted Django templates.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use minijinja::value::Value as JinjaValue;
use minijinja::{AutoEscape, Environment, UndefinedBehavior};
use rust_embed::RustEmbed;
use serde_json::{json, Value};

use crate::filters;
use crate::library_cache;
use crate::steam;

#[derive(RustEmbed)]
#[folder = "ui"]
struct Ui;

const CONTEXTS: &str = include_str!("../data/contexts.json");
const SEED: &str = include_str!("../data/seed.json");
const CSRF: &str = "lucienne-local-session";

pub struct Store {
    contexts: HashMap<String, Value>,
    pub products: HashMap<i64, Value>,
    pub user: Value,
    pub posts: Vec<Value>,
    pub channels: Vec<Value>,
    pub wishlist: Vec<i64>,
    pub library: Vec<i64>,
    pub comments: HashMap<i64, Vec<Value>>,
    pub reviews: Vec<Value>,
    next_id: i64,
}

#[derive(Clone)]
pub struct AppState {
    pub inner: Arc<Mutex<Store>>,
    env: Arc<Environment<'static>>,
}

fn system_today() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

fn build_environment(sources: &HashMap<String, String>) -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Chainable);
    env.set_auto_escape_callback(|name| {
        if name.ends_with(".html") {
            AutoEscape::Html
        } else {
            AutoEscape::None
        }
    });
    for (name, source) in sources {
        let template_name = name.trim_start_matches("templates/").to_string();
        let _ = env.add_template_owned(template_name, source.clone());
    }
    env.add_filter("ddefault", filters::ddefault);
    env.add_filter("ddefault_if_none", filters::ddefault_if_none);
    env.add_filter("truncatechars", filters::truncatechars);
    env.add_filter("truncatewords", filters::truncatewords);
    env.add_filter("floatformat", filters::floatformat);
    env.add_filter("yesno", filters::yesno);
    env.add_filter("pluralize", filters::pluralize);
    env.add_filter(
        "escapejs",
        |value: JinjaValue| -> Result<JinjaValue, minijinja::Error> {
            Ok(JinjaValue::from(filters::escapejs(value)?))
        },
    );
    env.add_filter("ddate", filters::ddate);
    env
}

impl Store {
    fn load() -> (Store, Environment<'static>) {
        let seed: Value = serde_json::from_str(SEED).expect("seed.json");
        let contexts: Value = serde_json::from_str(CONTEXTS).expect("contexts.json");

        let mut products = HashMap::new();
        if let Some(items) = seed.get("products").and_then(Value::as_array) {
            for product in items {
                if let Some(id) = product.get("id").and_then(Value::as_i64) {
                    products.insert(id, product.clone());
                }
            }
        }

        let mut sources = HashMap::new();
        for file in Ui::iter() {
            let name = file.as_ref();
            if let Some(source) = Ui::get(name) {
                if name.ends_with(".html") {
                    sources.insert(
                        name.to_string(),
                        String::from_utf8_lossy(&source.data).into_owned(),
                    );
                }
            }
        }

        let mut contexts_map = HashMap::new();
        if let Some(obj) = contexts.as_object() {
            for (key, value) in obj {
                contexts_map.insert(key.clone(), value.clone());
            }
        }

        let mut wishlist = Vec::new();
        let mut library = Vec::new();
        if let Some(seed_user) = seed.get("user") {
            if let Some(items) = seed_user.get("wishlist").and_then(Value::as_array) {
                for item in items {
                    if let Some(id) = item.get("product_id").and_then(Value::as_i64) {
                        wishlist.push(id);
                    }
                }
            }
            if let Some(items) = seed_user.get("library").and_then(Value::as_array) {
                for item in items {
                    if let Some(id) = item.get("product_id").and_then(Value::as_i64) {
                        library.push(id);
                    }
                }
            }
        }

        let posts = seed
            .get("posts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let channels = seed
            .get("channels")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut comments: HashMap<i64, Vec<Value>> = HashMap::new();
        if let Some(items) = contexts_map
            .get("community")
            .and_then(|ctx| ctx.get("comments"))
            .and_then(Value::as_array)
        {
            for comment in items {
                if let Some(post_id) = comment.get("post_id").and_then(Value::as_i64) {
                    comments.entry(post_id).or_default().push(json!({
                        "id": comment.get("id").cloned().unwrap_or(json!(0)),
                        "author_name": comment.get("author_name").cloned().unwrap_or(json!("kaichan")),
                        "author_avatar": comment.get("author_avatar").cloned().unwrap_or(json!("/static/dashboard/images/accsetup.jpg")),
                        "content": comment.get("content").cloned().unwrap_or(json!("")),
                        "created_at": "Just now",
                    }));
                }
            }
        }

        let user = contexts_map
            .get("dashboard")
            .and_then(|ctx| ctx.get("user"))
            .cloned()
            .unwrap_or(json!({}));

        let env = build_environment(&sources);
        (
            Store {
                contexts: contexts_map,
                products,
                user,
                posts,
                channels,
                wishlist,
                library,
                comments,
                reviews: Vec::new(),
                next_id: 1000,
            },
            env,
        )
    }

    fn hydrate(&self, mut context: Value) -> Value {
        for key in ["wishlist_items", "library_items", "submissions"] {
            if let Some(items) = context.get_mut(key).and_then(Value::as_array_mut) {
                for item in items.iter_mut() {
                    if let Some(obj) = item.as_object_mut() {
                        if let Some(pid) = obj.get("product").and_then(Value::as_i64) {
                            if let Some(product) = self.products.get(&pid) {
                                obj.insert("product".to_string(), product.clone());
                            }
                        }
                    }
                }
            }
        }
        let user = context
            .get("user")
            .cloned()
            .unwrap_or_else(|| self.user.clone());
        if let Some(obj) = context.as_object_mut() {
            obj.insert(
                "request".to_string(),
                json!({"path": "/", "user": user, "GET": {}, "is_ajax": false}),
            );
            obj.insert("messages".to_string(), json!([]));
            obj.insert("perms".to_string(), json!({}));
            obj.insert("csrf_token".to_string(), json!(CSRF));
        }
        context
    }

    /// Context for a single community post card. Callers must clone it out and
    /// render after releasing the lock, so rendering never blocks the store.
    fn post_context(&self, post: &Value) -> Value {
        json!({
            "post": post,
            "user": self.user,
            "request": {"path": "/community/", "user": self.user, "GET": {}},
            "messages": [],
            "perms": {},
            "csrf_token": CSRF,
        })
    }

    fn find_post(&mut self, post_id: i64) -> Option<&mut Value> {
        self.posts
            .iter_mut()
            .find(|post| post.get("id").and_then(Value::as_i64) == Some(post_id))
    }

    fn channel_for_slug(&self, slug: &str) -> Option<Value> {
        self.channels
            .iter()
            .find(|channel| channel.get("slug").and_then(Value::as_str) == Some(slug))
            .cloned()
    }
}

impl AppState {
    fn render(&self, template: &str, context: &Value) -> Result<String, String> {
        let tpl = self
            .env
            .get_template(template)
            .map_err(|err| format!("template {template}: {err}"))?;
        let html = tpl
            .render(context)
            .map_err(|err| format!("render {template}: {err}"))?;
        // minijinja escapes "/" in HTML; Django and the original templates do not.
        // The entity is harmless in markup but would corrupt URLs inside <script>.
        Ok(html.replace("&#x2f;", "/").replace("&#x2F;", "/"))
    }

    fn page(&self, key: &str) -> Result<String, String> {
        let template = if key.starts_with("detail-") {
            "dashboard/pages/game_detail.html".to_string()
        } else {
            match key {
                "developer" => "dashboard/pages/developer_panel.html".to_string(),
                "admin" => "dashboard/pages/admin_panel.html".to_string(),
                other => format!("dashboard/pages/{other}.html"),
            }
        };
        self.render_page_with(key, &template)
    }

    fn render_page_with(&self, key: &str, template: &str) -> Result<String, String> {
        let mut context = {
            let store = self.inner.lock().unwrap();
            let raw = store
                .contexts
                .get(key)
                .ok_or_else(|| format!("no context for {key}"))?
                .clone();
            store.hydrate(raw)
        };
        // The Library page shows what is installed in Steam, not the mocked
        // seed rows, so the scan replaces that context at render time.
        if key == "library" {
            apply_steam_library(&mut context);
        }
        self.render(template, &context)
    }

    fn post_html(&self, post: &Value) -> Result<String, String> {
        let context = {
            let store = self.inner.lock().unwrap();
            store.post_context(post)
        };
        self.render("dashboard/partials/community_post_card.html", &context)
    }
}

/// Snapshot of the saved library in `~/.lucienne` as the page expects it.
/// This only reads local files; pressing Refresh populates the cache.
fn steam_library_json() -> Value {
    let cache_dir = library_cache::cache_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let Some(cached) = library_cache::load() else {
        return json!({
            "available": false,
            "root": "",
            "cache_dir": cache_dir,
            "scanned_at": "",
            "games": [],
            "total_count": 0,
            "total_size": "0 B",
        });
    };

    let total_bytes: u64 = cached.games.iter().map(|game| game.size_on_disk).sum();
    let games: Vec<Value> = cached
        .games
        .iter()
        .map(|game| {
            let activity = if game.last_updated > 0 {
                format!("Updated {}", steam::date_from_unix(game.last_updated))
            } else if game.last_played > 0 {
                format!("Played {}", steam::date_from_unix(game.last_played))
            } else {
                "Installed".to_string()
            };
            let size = steam::format_size(game.size_on_disk);
            let local_cover = format!("/media/steam/{}", game.appid);
            // Once Refresh has cached the cover, the page never hits the CDN.
            let image_url = if library_cache::cover_file(game.appid).is_some() {
                local_cover.clone()
            } else {
                steam::cover_url(game.appid)
            };
            json!({
                "appid": game.appid,
                "title": game.name,
                "library": game.library,
                "size_bytes": game.size_on_disk,
                "meta": size,
                "status": "Installed",
                "updated": activity,
                "updated_at": steam::date_from_unix(game.last_updated),
                "last_played_at": steam::date_from_unix(game.last_played),
                "image_url": image_url,
                "header_url": steam::header_url(game.appid),
                "local_cover_url": local_cover,
                "favorite": false,
                "badge": "",
            })
        })
        .collect();
    let total_count = games.len();

    json!({
        "available": true,
        "root": cached.steam_root,
        "cache_dir": cache_dir,
        "scanned_at": steam::date_from_unix(cached.scanned_at),
        "games": games,
        "total_count": total_count,
        "total_size": steam::format_size(total_bytes),
    })
}

fn apply_steam_library(context: &mut Value) {
    let snapshot = steam_library_json();
    let Some(obj) = context.as_object_mut() else {
        return;
    };
    for (key, fallback) in [
        ("available", json!(false)),
        ("root", json!("")),
        ("cache_dir", json!("")),
        ("scanned_at", json!("")),
        ("games", json!([])),
        ("total_count", json!(0)),
        ("total_size", json!("0 B")),
    ] {
        obj.insert(
            key.to_string(),
            snapshot.get(key).cloned().unwrap_or(fallback),
        );
    }
    // The mocked hero banner has no real playtime behind it.
    obj.insert("now_playing".to_string(), Value::Null);
}

async fn steam_library() -> Json<Value> {
    Json(steam_library_json())
}

/// Scans Steam and writes the snapshot plus covers to `~/.lucienne`.
async fn steam_library_refresh() -> Response {
    match tokio::task::spawn_blocking(library_cache::refresh).await {
        Ok(Ok(_)) => Json(steam_library_json()).into_response(),
        Ok(Err(err)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": err })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// Serves portrait artwork, preferring the copy saved in `~/.lucienne` and
/// falling back to Steam's own `appcache/librarycache` folder.
async fn steam_media(AxumPath(appid): AxumPath<String>) -> Response {
    let Some(appid) = appid.parse::<u64>().ok() else {
        return (StatusCode::NOT_FOUND, "no cached artwork").into_response();
    };
    let Some(path) = library_cache::cover_file(appid).or_else(|| steam::cached_cover(appid)) else {
        return (StatusCode::NOT_FOUND, "no cached artwork").into_response();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return (StatusCode::NOT_FOUND, "no cached artwork").into_response();
    };
    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime.essence_str().to_string()),
            (header::CACHE_CONTROL, "public, max-age=3600".to_string()),
        ],
        bytes,
    )
        .into_response()
}

fn static_asset(path: &str) -> Response {
    let Some(file) = Ui::get(path) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime.essence_str().to_string()),
            // Assets are baked into the binary, so they never change at runtime.
            // Without this every navigation re-downloads ~760 KB of vendor JS.
            (
                header::CACHE_CONTROL,
                "public, max-age=31536000, immutable".to_string(),
            ),
        ],
        file.data.into_owned(),
    )
        .into_response()
}

fn page_response(state: &AppState, key: &str) -> Response {
    match state.page(key) {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            eprintln!("page error: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, err).into_response()
        }
    }
}

async fn index() -> Redirect {
    Redirect::to("/dashboard/")
}

async fn dashboard(State(state): State<AppState>) -> Response {
    page_response(&state, "dashboard")
}

async fn shop(State(state): State<AppState>) -> Response {
    page_response(&state, "shop")
}

async fn wishlist_page(State(state): State<AppState>) -> Response {
    page_response(&state, "wishlist")
}

async fn library_page(State(state): State<AppState>) -> Response {
    page_response(&state, "library")
}

async fn profile_page(State(state): State<AppState>) -> Response {
    page_response(&state, "profile")
}

async fn settings_page(State(state): State<AppState>) -> Response {
    page_response(&state, "settings")
}

async fn developer_page(State(state): State<AppState>) -> Response {
    page_response(&state, "developer")
}

async fn admin_page(State(state): State<AppState>) -> Response {
    page_response(&state, "admin")
}

// The converted pages still contain real <form method="post"> elements. This
// mock server does not persist them, but without a POST route axum answers 405
// with an empty body, which WebKit renders as a blank white window. Bounce the
// submission back to the page instead of letting it replace the app with that.
async fn profile_save() -> Redirect {
    Redirect::to("/profile/")
}

async fn settings_save() -> Redirect {
    Redirect::to("/settings/")
}

async fn admin_action() -> Redirect {
    Redirect::to("/settings/admin/")
}

async fn game_detail(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    let key = format!("detail-{id}");
    let known = {
        let store = state.inner.lock().unwrap();
        store.contexts.contains_key(&key)
    };
    if !known {
        return (StatusCode::NOT_FOUND, "Unknown product").into_response();
    }
    page_response(&state, &key)
}

async fn community_page(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let is_ajax = headers
        .get("x-requested-with")
        .and_then(|value| value.to_str().ok())
        == Some("XMLHttpRequest");
    if !is_ajax {
        return page_response(&state, "community");
    }

    let channel = params.get("channel").cloned().unwrap_or_default();
    let feed = params.get("feed").cloned().unwrap_or_default();
    let search = params
        .get("search")
        .cloned()
        .unwrap_or_else(|| params.get("q").cloned().unwrap_or_default());
    let needle = search.to_lowercase();

    // Collect the matching posts, then release the store lock before
    // rendering so template work never serializes API requests.
    let matching: Vec<Value> = {
        let store = state.inner.lock().unwrap();
        store
            .posts
            .iter()
            .filter(|post| {
                if !channel.is_empty() {
                    let slug = post
                        .get("channel")
                        .and_then(|c| c.get("slug"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if slug != channel {
                        return false;
                    }
                }
                if feed == "following" {
                    return false;
                }
                if !needle.is_empty() {
                    let text = format!(
                        "{} {}",
                        post.get("title").and_then(Value::as_str).unwrap_or(""),
                        post.get("content").and_then(Value::as_str).unwrap_or("")
                    )
                    .to_lowercase();
                    return text.contains(&needle);
                }
                true
            })
            .cloned()
            .collect()
    };

    let mut html = String::new();
    for post in &matching {
        if let Ok(card) = state.post_html(post) {
            html.push_str(&card);
        }
    }
    Json(json!({"posts_html": html, "has_next": false})).into_response()
}

async fn static_files(AxumPath(path): AxumPath<String>) -> Response {
    static_asset(&format!("static/{path}"))
}

async fn wishlist_list(State(state): State<AppState>) -> Json<Value> {
    let store = state.inner.lock().unwrap();
    Json(json!({"product_ids": store.wishlist}))
}

async fn wishlist_toggle(State(state): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
    let Some(product_id) = body.get("product_id").and_then(Value::as_i64) else {
        return Json(json!({"error": "product_id required"}));
    };
    let mut store = state.inner.lock().unwrap();
    let wished = if let Some(index) = store.wishlist.iter().position(|id| *id == product_id) {
        store.wishlist.remove(index);
        false
    } else {
        store.wishlist.push(product_id);
        true
    };
    Json(json!({"wished": wished}))
}

async fn check_ownership(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Value> {
    let product_id = params
        .get("product_id")
        .and_then(|value| value.parse::<i64>().ok());
    let store = state.inner.lock().unwrap();
    let owned = product_id
        .map(|id| store.library.contains(&id))
        .unwrap_or(false);
    Json(json!({"owned": owned}))
}

async fn checkout(State(state): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
    let ids: Vec<i64> = body
        .get("product_ids")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default();
    let mut store = state.inner.lock().unwrap();
    let mut added = 0;
    for id in ids {
        if !store.library.contains(&id) {
            store.library.push(id);
            added += 1;
        }
        store.wishlist.retain(|wished| *wished != id);
    }
    Json(json!({"success": true, "added": added}))
}

async fn reviews_list(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Value> {
    let product_id = params
        .get("product_id")
        .and_then(|value| value.parse::<i64>().ok());
    let store = state.inner.lock().unwrap();
    let matching: Vec<Value> = store
        .reviews
        .iter()
        .filter(|review| review.get("product_id").and_then(Value::as_i64) == product_id)
        .cloned()
        .collect();
    let positive = matching
        .iter()
        .filter(|review| review.get("positive").and_then(Value::as_bool) == Some(true))
        .count();
    let pct = if matching.is_empty() {
        0
    } else {
        (positive * 100) / matching.len()
    };
    Json(json!({
        "reviews": matching,
        "total": store.reviews.len(),
        "positive_pct": pct,
        "summary": "",
    }))
}

async fn review_submit(State(state): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
    let mut store = state.inner.lock().unwrap();
    store.next_id += 1;
    let review = json!({
        "id": store.next_id,
        "product_id": body.get("product_id").cloned().unwrap_or(json!(0)),
        "author": store.user.get("display_name").and_then(Value::as_str)
            .or_else(|| store.user.get("username").and_then(Value::as_str))
            .unwrap_or("kaichan"),
        "avatar": store.user.get("avatar_url").cloned().unwrap_or(json!("/static/dashboard/images/accsetup.jpg")),
        "date": system_today(),
        "hours": Value::Null,
        "lang": body.get("lang").cloned().unwrap_or(json!("en")),
        "positive": body.get("positive").cloned().unwrap_or(json!(true)),
        "text": body.get("text").cloned().unwrap_or(json!("")),
        "helpful": 0,
    });
    store.reviews.insert(0, review.clone());
    Json(json!({"success": true, "review": review}))
}

async fn community_comments(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Value> {
    let post_id = params
        .get("post_id")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let store = state.inner.lock().unwrap();
    let comments = store.comments.get(&post_id).cloned().unwrap_or_default();
    Json(json!({"comments": comments}))
}

async fn community_comment(State(state): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
    let Some(post_id) = body.get("post_id").and_then(Value::as_i64) else {
        return Json(json!({"error": "post_id required"}));
    };
    let content = body
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Json(json!({"error": "Comment cannot be empty."}));
    }
    let mut store = state.inner.lock().unwrap();
    store.next_id += 1;
    let comment = json!({
        "id": store.next_id,
        "author_name": store.user.get("username").and_then(Value::as_str).unwrap_or("kaichan"),
        "author_avatar": store.user.get("avatar_url").cloned().unwrap_or(json!("/static/dashboard/images/accsetup.jpg")),
        "content": content,
        "created_at": "Just now",
    });
    let entry = store.comments.entry(post_id).or_default();
    entry.push(comment.clone());
    let total = entry.len();
    if let Some(post) = store.find_post(post_id) {
        if let Some(obj) = post.as_object_mut() {
            obj.insert("comment_count".to_string(), json!(total));
        }
    }
    Json(json!({"success": true, "comment": comment, "total_comments": total}))
}

async fn community_share(State(state): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
    let Some(post_id) = body.get("post_id").and_then(Value::as_i64) else {
        return Json(json!({"error": "post_id required"}));
    };
    let mut store = state.inner.lock().unwrap();
    let mut count = 0;
    if let Some(post) = store.find_post(post_id) {
        if let Some(obj) = post.as_object_mut() {
            count = obj.get("share_count").and_then(Value::as_i64).unwrap_or(0) + 1;
            obj.insert("share_count".to_string(), json!(count));
        }
    }
    Json(json!({"success": true, "count": count}))
}

async fn community_vote(State(state): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
    let Some(option_id) = body.get("option_id").and_then(Value::as_i64) else {
        return Json(json!({"error": "option_id required"}));
    };
    let mut store = state.inner.lock().unwrap();
    let mut response = json!({"error": "Option not found"});

    for post in store.posts.iter_mut() {
        let Some(options) = post.get_mut("poll_options").and_then(Value::as_array_mut) else {
            continue;
        };
        if !options
            .iter()
            .any(|option| option.get("id").and_then(Value::as_i64) == Some(option_id))
        {
            continue;
        }
        let mut total = 0;
        for option in options.iter_mut() {
            let is_chosen = option.get("id").and_then(Value::as_i64) == Some(option_id);
            if let Some(obj) = option.as_object_mut() {
                let votes = obj.get("votes").and_then(Value::as_i64).unwrap_or(0);
                let votes = if is_chosen {
                    votes + 1
                } else if obj.get("is_selected").and_then(Value::as_bool) == Some(true) {
                    (votes - 1).max(0)
                } else {
                    votes
                };
                obj.insert("votes".to_string(), json!(votes));
                obj.insert("is_selected".to_string(), json!(is_chosen));
            }
        }
        for option in options.iter() {
            total += option.get("votes").and_then(Value::as_i64).unwrap_or(0);
        }
        for option in options.iter_mut() {
            if let Some(obj) = option.as_object_mut() {
                let votes = obj.get("votes").and_then(Value::as_i64).unwrap_or(0);
                let percentage = if total > 0 { (votes * 100) / total } else { 0 };
                obj.insert("percentage".to_string(), json!(percentage));
            }
        }
        response = json!({"options": options.clone(), "total_votes": total});
        break;
    }
    Json(response)
}

async fn community_create_post(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut store = state.inner.lock().unwrap();
    store.next_id += 1;
    let id = store.next_id;
    let slug = body.get("channel").and_then(Value::as_str).unwrap_or("");
    let channel = store.channel_for_slug(slug);
    let post_type = body
        .get("post_type")
        .and_then(Value::as_str)
        .unwrap_or("discussion")
        .to_string();
    let title = body.get("title").and_then(Value::as_str).unwrap_or("");
    let content = body.get("content").and_then(Value::as_str).unwrap_or("");
    let images: Vec<Value> = body
        .get("image_urls")
        .and_then(Value::as_array)
        .map(|urls| {
            urls.iter()
                .filter_map(Value::as_str)
                .map(|url| json!({"url": url}))
                .collect()
        })
        .unwrap_or_default();

    let mut poll_obj = Value::Null;
    let mut poll_options: Vec<Value> = Vec::new();
    if post_type == "poll" {
        poll_obj = json!({
            "question": body.get("poll_question").cloned().unwrap_or(json!("")),
            "time_left": "7 days left",
        });
        let options = body
            .get("poll_options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (index, option) in options.iter().enumerate() {
            poll_options.push(json!({
                "id": id * 10 + index as i64,
                "text": option,
                "votes": 0,
                "percentage": 0,
                "is_selected": false,
            }));
        }
    }

    let post = json!({
        "id": id,
        "post_type": post_type,
        "title": title,
        "content": content,
        "author_name": store.user.get("username").and_then(Value::as_str).unwrap_or("kaichan"),
        "author_avatar": store.user.get("avatar_url").cloned().unwrap_or(json!("/static/dashboard/images/accsetup.jpg")),
        "author_badge": "",
        "channel": channel,
        "channel_id": 0,
        "is_pinned": false,
        "time_ago": "Just now",
        "visibility": "Public",
        "reaction_count": 0,
        "comment_count": 0,
        "share_count": 0,
        "view_count": 0,
        "bookmark_count": 0,
        "created_at": format!("{}T00:00:00+00:00", system_today()),
        "images": images,
        "tags": [],
        "reactions": [],
        "is_owner": true,
        "poll_obj": poll_obj,
        "poll_options": poll_options,
        "poll_voted": false,
        "poll_selected_id": Value::Null,
        "poll_total": 0,
    });

    store.posts.insert(0, post.clone());
    drop(store);
    let html = state.post_html(&post).unwrap_or_default();
    Json(json!({"success": true, "post_html": html}))
}

async fn community_edit_post(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let Some(post_id) = body.get("post_id").and_then(Value::as_i64) else {
        return Json(json!({"error": "post_id required"}));
    };
    let mut store = state.inner.lock().unwrap();
    let mut updated = None;
    if let Some(post) = store.find_post(post_id) {
        if let Some(obj) = post.as_object_mut() {
            if let Some(title) = body.get("title").and_then(Value::as_str) {
                obj.insert("title".to_string(), json!(title));
            }
            if let Some(content) = body.get("content").and_then(Value::as_str) {
                obj.insert("content".to_string(), json!(content));
            }
        }
        updated = Some(post.clone());
    }
    drop(store);
    let html = updated
        .map(|post| state.post_html(&post).unwrap_or_default())
        .unwrap_or_default();
    Json(json!({"success": true, "post_html": html}))
}

async fn community_delete_post(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let Some(post_id) = body.get("post_id").and_then(Value::as_i64) else {
        return Json(json!({"error": "post_id required"}));
    };
    let mut store = state.inner.lock().unwrap();
    store
        .posts
        .retain(|post| post.get("id").and_then(Value::as_i64) != Some(post_id));
    Json(json!({"success": true}))
}

async fn community_follow_game() -> Json<Value> {
    Json(json!({"success": true}))
}

async fn api_fallback() -> Json<Value> {
    Json(json!({"success": true}))
}

pub fn router() -> Router {
    let (store, env) = Store::load();
    let state = AppState {
        inner: Arc::new(Mutex::new(store)),
        env: Arc::new(env),
    };

    Router::new()
        .route("/", get(index))
        .route("/dashboard/", get(dashboard))
        .route("/shop/", get(shop))
        .route("/shop/:id/", get(game_detail))
        .route("/community/", get(community_page))
        .route("/wishlist/", get(wishlist_page))
        .route("/library/", get(library_page))
        .route("/profile/", get(profile_page).post(profile_save))
        .route("/settings/", get(settings_page).post(settings_save))
        .route("/settings/developer/", get(developer_page))
        .route("/settings/admin/", get(admin_page).post(admin_action))
        .route("/api/steam/library/", get(steam_library))
        .route("/api/steam/library/refresh/", post(steam_library_refresh))
        .route("/media/steam/:appid", get(steam_media))
        .route("/api/wishlist/list/", get(wishlist_list))
        .route("/api/wishlist/toggle/", post(wishlist_toggle))
        .route("/api/shop/check-ownership/", get(check_ownership))
        .route("/api/shop/checkout/", post(checkout))
        .route("/api/reviews/", get(reviews_list))
        .route("/api/reviews/submit/", post(review_submit))
        .route("/api/community/comments/", get(community_comments))
        .route("/api/community/comment/", post(community_comment))
        .route("/api/community/share/", post(community_share))
        .route("/api/community/vote/", post(community_vote))
        .route("/api/community/post/", post(community_create_post))
        .route("/api/community/post/edit/", post(community_edit_post))
        .route("/api/community/post/delete/", post(community_delete_post))
        .route("/api/community/games/follow/", post(community_follow_game))
        .route("/static/*path", get(static_files))
        .fallback(api_fallback)
        .with_state(state)
}

pub fn start() -> std::io::Result<SocketAddr> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?;
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
            if let Err(err) = axum::serve(listener, router()).await {
                eprintln!("server stopped: {err}");
            }
        });
    });
    Ok(address)
}
