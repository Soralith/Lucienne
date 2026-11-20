//! Persistent snapshot of the Steam library in `~/.lucienne`.
//!
//! Scanning Steam and fetching artwork only happens when the user presses
//! Refresh on the Library page. Every launch then reads `library.json` and the
//! cover files next to it, so start-up never touches Steam or the network.

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::steam::{self, InstalledGame};

const LIBRARY_FILE: &str = "library.json";
const COVERS_DIR: &str = "covers";

/// Covers are small; cap the read so a bad response cannot fill memory.
const MAX_COVER_BYTES: u64 = 8 * 1024 * 1024;
const COVER_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Serialize, Deserialize)]
pub struct CachedLibrary {
    pub scanned_at: u64,
    pub steam_root: String,
    pub games: Vec<InstalledGame>,
}

pub fn cache_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".lucienne"))
}

/// The saved snapshot, or `None` when the library has never been scanned.
pub fn load() -> Option<CachedLibrary> {
    let path = cache_dir()?.join(LIBRARY_FILE);
    let source = fs::read_to_string(path).ok()?;
    serde_json::from_str(&source).ok()
}

/// Scan Steam, cache any missing covers, then persist the snapshot.
pub fn refresh() -> Result<CachedLibrary, String> {
    let library = steam::installed_games().ok_or("Steam library not found")?;
    let dir = cache_dir().ok_or("Could not locate the home directory")?;
    let covers = dir.join(COVERS_DIR);
    fs::create_dir_all(&covers).map_err(|err| format!("create {}: {err}", covers.display()))?;

    for game in &library.games {
        cache_cover(game.appid, &covers);
    }

    let snapshot = CachedLibrary {
        scanned_at: now_seconds(),
        steam_root: library.root.display().to_string(),
        games: library.games,
    };
    save(&dir, &snapshot)?;
    Ok(snapshot)
}

/// Local cover file for an app, copied into the cache during Refresh.
pub fn cover_file(appid: u64) -> Option<PathBuf> {
    let covers = cache_dir()?.join(COVERS_DIR);
    let stem = appid.to_string();
    fs::read_dir(covers)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .map(|name| name == stem)
                    .unwrap_or(false)
        })
}

/// Prefer the artwork Steam already downloaded; fall back to the CDN once.
fn cache_cover(appid: u64, covers: &Path) {
    if cover_file(appid).is_some() {
        return;
    }
    if let Some(source) = steam::cached_cover(appid) {
        if let Ok(bytes) = fs::read(&source) {
            let extension = source
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("jpg");
            let target = covers.join(format!("{appid}.{extension}"));
            let _ = fs::write(target, bytes);
            return;
        }
    }
    download_cover(appid, covers);
}

fn download_cover(appid: u64, covers: &Path) {
    let Ok(response) = ureq::get(&steam::cover_url(appid))
        .timeout(COVER_TIMEOUT)
        .call()
    else {
        return;
    };
    let mut bytes = Vec::new();
    if response
        .into_reader()
        .take(MAX_COVER_BYTES)
        .read_to_end(&mut bytes)
        .is_ok()
        && !bytes.is_empty()
    {
        let _ = fs::write(covers.join(format!("{appid}.jpg")), bytes);
    }
}

fn save(dir: &Path, snapshot: &CachedLibrary) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
    let json = serde_json::to_string_pretty(snapshot)
        .map_err(|err| format!("serialize library: {err}"))?;
    // Write to a sibling file first so an interrupted refresh cannot leave a
    // truncated `library.json` behind.
    let temp = dir.join(format!("{LIBRARY_FILE}.tmp"));
    fs::write(&temp, json).map_err(|err| format!("write {}: {err}", temp.display()))?;
    fs::rename(&temp, dir.join(LIBRARY_FILE))
        .map_err(|err| format!("replace {}: {err}", LIBRARY_FILE))
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
