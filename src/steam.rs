//! Reads games installed in the local Steam library.
//!
//! Steam keeps one plain-text VDF manifest per installed app
//! (`steamapps/appmanifest_<appid>.acf`) and lists every library folder in
//! `steamapps/libraryfolders.vdf`. Reading those files needs no login and no
//! Web API key, which is why the Library page uses them instead of the API.

use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bit 4 of `StateFlags` means the app is fully installed.
const FULLY_INSTALLED: u64 = 4;

/// Steamworks Common Redistributables is installed by Steam, not by the user.
const SKIPPED_APPIDS: [u64; 1] = [228980];

/// Compatibility tools and runtimes appear in the manifest folder but are not
/// games, so they would only add empty cards to the Library grid.
const SKIPPED_PREFIXES: [&str; 2] = ["Proton", "Steam Linux Runtime"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledGame {
    pub appid: u64,
    pub name: String,
    pub size_on_disk: u64,
    pub last_updated: u64,
    pub last_played: u64,
    pub library: String,
}

impl InstalledGame {
    /// Timestamp of the most recent activity, used for the "recent" ordering.
    pub fn last_active(&self) -> u64 {
        self.last_updated.max(self.last_played)
    }
}

pub struct SteamLibrary {
    pub root: PathBuf,
    pub games: Vec<InstalledGame>,
}

pub fn cover_url(appid: u64) -> String {
    format!("https://cdn.akamai.steamstatic.com/steam/apps/{appid}/library_600x900.jpg")
}

pub fn header_url(appid: u64) -> String {
    format!("https://cdn.akamai.steamstatic.com/steam/apps/{appid}/header.jpg")
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Formats a Unix timestamp as `YYYY-MM-DD`, or an empty string for 0.
pub fn date_from_unix(seconds: u64) -> String {
    if seconds == 0 {
        return String::new();
    }
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{day:02}")
}

/// Scans every Steam library folder. Returns `None` when Steam is not installed,
/// and an empty game list when Steam exists but no game is installed.
///
/// `LUCIENNE_STEAM_ROOT` overrides the Steam install directory, which also makes
/// the scanner testable and helps with non-standard installs.
pub fn installed_games() -> Option<SteamLibrary> {
    let root = install_root()?;

    let mut games = Vec::new();
    let mut seen = HashSet::new();
    for library_root in library_roots(&root) {
        let steamapps = library_root.join("steamapps");
        let Ok(entries) = fs::read_dir(&steamapps) else {
            continue;
        };
        let library = library_label(&library_root);
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                continue;
            };
            if !(file_name.starts_with("appmanifest_") && file_name.ends_with(".acf")) {
                continue;
            }
            let Ok(source) = fs::read_to_string(entry.path()) else {
                continue;
            };
            let Some(mut game) = parse_manifest(&source) else {
                continue;
            };
            if !seen.insert(game.appid) {
                continue;
            }
            game.library = library.clone();
            games.push(game);
        }
    }

    games.sort_by(|a, b| {
        b.last_active()
            .cmp(&a.last_active())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Some(SteamLibrary { root, games })
}

/// The Steam client install directory, where the app cache also lives.
pub fn install_root() -> Option<PathBuf> {
    candidate_roots()
        .into_iter()
        .find(|path| path.join("steamapps").is_dir())
}

/// Best locally cached portrait artwork for an app.
///
/// Steam stores library art under `appcache/librarycache/<appid>/`, but the
/// filenames differ between games: stable names (`library_600x900.jpg`) for
/// released titles and hashed folders containing `library_capsule.jpg` for
/// newer ones. Prefer the known portrait names, then fall back to any cached
/// image whose dimensions have a 2:3-ish aspect ratio.
pub fn cached_cover(appid: u64) -> Option<PathBuf> {
    let root = install_root()?;
    let cache = root.join("appcache/librarycache").join(appid.to_string());
    if !cache.is_dir() {
        return None;
    }

    let mut best: Option<(u32, PathBuf)> = None;
    let mut images = Vec::new();
    collect_images(&cache, 0, &mut images);
    for path in images {
        let score = cover_score(&path);
        if score == 0 {
            continue;
        }
        if best.as_ref().map(|(top, _)| score > *top).unwrap_or(true) {
            best = Some((score, path));
        }
    }
    best.map(|(_, path)| path)
}

fn collect_images(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    if depth > 2 || out.len() >= 64 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_images(&path, depth + 1, out);
            continue;
        }
        let is_image = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "jpg" | "jpeg" | "png" | "webp"
                )
            })
            .unwrap_or(false);
        if is_image {
            out.push(path);
        }
    }
}

fn cover_score(path: &Path) -> u32 {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let area = image_dimensions(path)
        .map(|(width, height)| width.saturating_mul(height) / 1000)
        .unwrap_or(0)
        .min(999);

    if name.contains("library_600x900") {
        return 4000 + area;
    }
    if name.contains("library_capsule") {
        return 3000 + area;
    }
    match image_dimensions(path) {
        // 2:3 portrait artwork (300x450, 600x900, ...), while headers, heroes,
        // logos and tiny icons are ruled out.
        Some((width, height)) if width >= 200 && (0.55..=0.80).contains(&ratio(width, height)) => {
            1000 + area
        }
        _ => 0,
    }
}

fn ratio(width: u32, height: u32) -> f32 {
    if height == 0 {
        return 0.0;
    }
    width as f32 / height as f32
}

fn image_dimensions(path: &Path) -> Option<(u32, u32)> {
    let data = fs::read(path).ok()?;
    dimensions(&data)
}

fn dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.starts_with(&[0xFF, 0xD8]) {
        jpeg_dimensions(data)
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_dimensions(data)
    } else {
        None
    }
}

fn png_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 24 {
        return None;
    }
    let width = u32::from_be_bytes(data[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(data[20..24].try_into().ok()?);
    Some((width, height))
}

fn jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 8 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        i += 2;
        // Standalone markers carry no length field.
        if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }
        let length = ((data[i] as usize) << 8) | data[i + 1] as usize;
        if length < 2 || i + length > data.len() {
            return None;
        }
        let is_sof =
            (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC;
        if is_sof {
            let height = ((data[i + 3] as u32) << 8) | data[i + 4] as u32;
            let width = ((data[i + 5] as u32) << 8) | data[i + 6] as u32;
            return Some((width, height));
        }
        i += length;
    }
    None
}

fn candidate_roots() -> Vec<PathBuf> {
    if let Some(custom) = env::var_os("LUCIENNE_STEAM_ROOT") {
        return vec![PathBuf::from(custom)];
    }

    let mut roots = Vec::new();
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if let Some(home) = &home {
        roots.push(home.join(".local/share/Steam"));
        roots.push(home.join(".steam/steam"));
        roots.push(home.join(".steam/root"));
        roots.push(home.join(".var/app/com.valvesoftware.Steam/data/Steam"));
        roots.push(home.join("Library/Application Support/Steam"));
    }
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(dir) = env::var_os(var) {
            roots.push(PathBuf::from(dir).join("Steam"));
        }
    }

    let mut seen = HashSet::new();
    roots.retain(|path| seen.insert(fs::canonicalize(path).unwrap_or_else(|_| path.clone())));
    roots
}

/// The install root plus every additional drive listed in `libraryfolders.vdf`.
fn library_roots(root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![root.to_path_buf()];
    let source = fs::read_to_string(root.join("steamapps/libraryfolders.vdf")).unwrap_or_default();
    if !source.is_empty() {
        if let Some(tree) = parse_vdf(&source) {
            let mut paths = Vec::new();
            collect_paths(&tree, &mut paths);
            for path in paths {
                if path.join("steamapps").is_dir() && !roots.contains(&path) {
                    roots.push(path);
                }
            }
        }
    }
    roots
}

fn library_label(library_root: &Path) -> String {
    library_root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| library_root.display().to_string())
}

fn collect_paths(node: &Vdf, out: &mut Vec<PathBuf>) {
    let Vdf::Branch(map) = node else {
        return;
    };
    for (key, value) in map {
        match (key.as_str(), value) {
            (key, Vdf::Text(path)) if key.eq_ignore_ascii_case("path") => {
                out.push(PathBuf::from(path));
            }
            _ => collect_paths(value, out),
        }
    }
}

fn parse_manifest(source: &str) -> Option<InstalledGame> {
    let tree = parse_vdf(source)?;
    let app = tree.get("AppState")?;
    let appid: u64 = app.text("appid")?.trim().parse().ok()?;
    let name = app.text("name")?.trim().to_string();

    // "App_1234" is what Steam writes when it has no store metadata for an app.
    if name.is_empty()
        || name.starts_with("App_")
        || name == "Steamworks Common Redistributables"
        || SKIPPED_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        || SKIPPED_APPIDS.contains(&appid)
    {
        return None;
    }
    if app.number("StateFlags") & FULLY_INSTALLED == 0 {
        return None;
    }

    Some(InstalledGame {
        appid,
        name,
        size_on_disk: app.number("SizeOnDisk"),
        last_updated: app.number("LastUpdated"),
        last_played: app.number("LastPlayed"),
        library: String::new(),
    })
}

// ===== Minimal VDF (Valve KeyValues) parser =====

enum Vdf {
    Text(String),
    Branch(BTreeMap<String, Vdf>),
}

impl Vdf {
    fn get(&self, key: &str) -> Option<&Vdf> {
        let Vdf::Branch(map) = self else {
            return None;
        };
        map.iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    }

    fn text(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Vdf::Text(text) => Some(text),
            Vdf::Branch(_) => None,
        }
    }

    fn number(&self, key: &str) -> u64 {
        self.text(key)
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0)
    }
}

enum Token {
    Text(String),
    Open,
    Close,
}

fn tokenize(source: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            ch if ch.is_whitespace() => {}
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '/' if chars.peek() == Some(&'/') => {
                while let Some(&next) = chars.peek() {
                    if next == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            '"' => {
                let mut text = String::new();
                while let Some(ch) = chars.next() {
                    match ch {
                        '"' => break,
                        '\\' => {
                            if let Some(escaped) = chars.next() {
                                text.push(escaped);
                            }
                        }
                        other => text.push(other),
                    }
                }
                tokens.push(Token::Text(text));
            }
            other => {
                let mut text = String::from(other);
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() || next == '{' || next == '}' || next == '"' {
                        break;
                    }
                    text.push(next);
                    chars.next();
                }
                tokens.push(Token::Text(text));
            }
        }
    }
    tokens
}

fn parse_vdf(source: &str) -> Option<Vdf> {
    let tokens = tokenize(source);
    let mut pos = 0;
    parse_branch(&tokens, &mut pos).map(Vdf::Branch)
}

fn parse_branch(tokens: &[Token], pos: &mut usize) -> Option<BTreeMap<String, Vdf>> {
    let mut map = BTreeMap::new();
    while let Some(token) = tokens.get(*pos) {
        match token {
            Token::Close => {
                *pos += 1;
                return Some(map);
            }
            Token::Open => *pos += 1,
            Token::Text(key) => {
                let key = key.clone();
                *pos += 1;
                match tokens.get(*pos) {
                    Some(Token::Open) => {
                        *pos += 1;
                        let branch = parse_branch(tokens, pos)?;
                        map.insert(key, Vdf::Branch(branch));
                    }
                    Some(Token::Text(value)) => {
                        map.insert(key, Vdf::Text(value.clone()));
                        *pos += 1;
                    }
                    _ => return Some(map),
                }
            }
        }
    }
    Some(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
"AppState"
{
	"appid"		"431960"
	"name"		"Wallpaper Engine"
	"StateFlags"		"4"
	"LastUpdated"		"1785389626"
	"LastPlayed"		"1794296583"
	"SizeOnDisk"		"834746493"
}
"#;

    #[test]
    fn parses_an_installed_manifest() {
        let game = parse_manifest(MANIFEST).expect("manifest should parse");
        assert_eq!(game.appid, 431960);
        assert_eq!(game.name, "Wallpaper Engine");
        assert_eq!(game.size_on_disk, 834_746_493);
        assert_eq!(game.last_updated, 1_785_389_626);
    }

    #[test]
    fn skips_placeholder_and_incomplete_apps() {
        let placeholder = MANIFEST.replace("Wallpaper Engine", "App_431960");
        assert!(parse_manifest(&placeholder).is_none());

        let incomplete = MANIFEST.replace(r#""StateFlags"		"4""#, r#""StateFlags"		"1026""#);
        assert!(parse_manifest(&incomplete).is_none());

        for tool in ["Proton Experimental", "Steam Linux Runtime 3.0 (sniper)"] {
            let manifest = MANIFEST.replace("Wallpaper Engine", tool);
            assert!(parse_manifest(&manifest).is_none(), "{tool} is not a game");
        }
    }

    #[test]
    fn finds_library_paths() {
        let source = r#"
"libraryfolders"
{
	"0" { "path" "/home/user/.local/share/Steam" }
	"1" { "path" "/mnt/games/SteamLibrary" }
}
"#;
        let tree = parse_vdf(source).expect("vdf should parse");
        let mut paths = Vec::new();
        collect_paths(&tree, &mut paths);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/home/user/.local/share/Steam"),
                PathBuf::from("/mnt/games/SteamLibrary")
            ]
        );
    }

    #[test]
    fn reads_image_dimensions() {
        let mut png = vec![0u8; 24];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[16..20].copy_from_slice(&300u32.to_be_bytes());
        png[20..24].copy_from_slice(&450u32.to_be_bytes());
        assert_eq!(dimensions(&png), Some((300, 450)));

        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        jpeg.extend_from_slice(&450u16.to_be_bytes());
        jpeg.extend_from_slice(&300u16.to_be_bytes());
        jpeg.push(0x03);
        // Pad out the declared SOF segment length (17 bytes).
        jpeg.extend_from_slice(&[0; 9]);
        assert_eq!(dimensions(&jpeg), Some((300, 450)));

        assert_eq!(dimensions(b"not an image"), None);
    }

    #[test]
    fn formats_sizes_and_dates() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(834_746_493), "796.1 MB");
        assert_eq!(format_size(22_141_113_242), "20.6 GB");
        assert_eq!(date_from_unix(1_785_389_626), "2026-07-30");
        assert_eq!(date_from_unix(0), "");
    }
}
