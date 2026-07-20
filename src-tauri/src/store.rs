//! On-disk state: connection profiles, terminal history, saved queries.
//!
//! Nothing here holds a secret — passwords live in the OS keychain
//! (`crate::secrets`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::db::connect::Profile;
use crate::error::{AppError, Result};

/// Keep history bounded; the sidebar only ever shows the tail.
const HISTORY_CAP: usize = 1000;

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    /// Locate the per-user config and data directories and create them, along
    /// with the saved-queries subdirectory, so everything below can assume they
    /// exist. Called once at startup; fails only if the OS reports no home.
    ///
    /// # Arguments
    /// None.
    ///
    /// # Returns
    /// `Result<Self>` — the two resolved roots, both guaranteed to exist on
    /// disk. `Err(AppError::Storage)` when the platform has no config or data
    /// directory, `Err(AppError::Io)` if either cannot be created.
    pub fn resolve() -> Result<Self> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| AppError::Storage("no config directory".into()))?
            .join("pgscope");
        let data_dir = dirs::data_dir()
            .ok_or_else(|| AppError::Storage("no data directory".into()))?
            .join("pgscope");
        std::fs::create_dir_all(&config_dir)?;
        std::fs::create_dir_all(&data_dir)?;
        std::fs::create_dir_all(data_dir.join("saved_queries"))?;
        Ok(Self {
            config_dir,
            data_dir,
        })
    }

    // These accessors are the single place each on-disk filename is decided.
    // Nothing else in the crate joins a literal onto `config_dir`/`data_dir`, so
    // moving or renaming a file is a one-line change here, and the tests can
    // point the whole store at a temporary directory via `paths_in`.

    /// A single JSON array of every saved connection profile.
    ///
    /// # Arguments
    /// None (`&self`: the resolved config and data roots).
    ///
    /// # Returns
    /// `PathBuf` — `profiles.json` under the config directory. The file need
    /// not exist; the loaders treat a missing one as "no profiles yet".
    pub fn profiles(&self) -> PathBuf {
        self.config_dir.join("profiles.json")
    }

    /// One JSON object per line, appended to as the terminal is used.
    ///
    /// # Arguments
    /// None (`&self`: the resolved config and data roots).
    ///
    /// # Returns
    /// `PathBuf` — `history.jsonl` under the data directory. May be absent
    /// until the first command is recorded.
    pub fn history(&self) -> PathBuf {
        self.data_dir.join("history.jsonl")
    }

    /// A directory of `.sql` files, possibly nested — not a single file.
    ///
    /// # Arguments
    /// None (`&self`: the resolved config and data roots).
    ///
    /// # Returns
    /// `PathBuf` — the `saved_queries` directory under the data root. Created
    /// by [`Paths::resolve`], and the containment root every saved-query path
    /// is checked against.
    pub fn saved_queries(&self) -> PathBuf {
        self.data_dir.join("saved_queries")
    }

    /// Card positions for the Relationships diagram, across every database the
    /// user connects to — the layouts are keyed by schema and table, not by
    /// profile.
    ///
    /// # Arguments
    /// None (`&self`: the resolved config and data roots).
    ///
    /// # Returns
    /// `PathBuf` — `er_layout.json` under the data directory. Absent until the
    /// first card is dragged.
    pub fn er_layout(&self) -> PathBuf {
        self.data_dir.join("er_layout.json")
    }

    /// Grid column widths and order, capped at `GRID_LAYOUT_CAP` tables.
    ///
    /// # Arguments
    /// None (`&self`: the resolved config and data roots).
    ///
    /// # Returns
    /// `PathBuf` — `grid_layout.json` under the data directory. Absent until a
    /// column is first resized or reordered.
    pub fn grid_layout(&self) -> PathBuf {
        self.data_dir.join("grid_layout.json")
    }
}

// ------------------------------- profiles -------------------------------

/// Every saved connection profile; empty on first run.
///
/// Unlike history and the layouts, a corrupt file errors rather than degrading
/// to "none". Returning an empty list would look to the user like their saved
/// connections had vanished, and the next [`upsert_profile`] would then write
/// that empty list back over the file they could otherwise have repaired by hand.
///
/// # Arguments
/// * `paths` — `&Paths`: roots for the config and data directories.
///
/// # Returns
/// `Result<Vec<Profile>>` — the profiles in file order; empty when the file is
/// absent, which is first run rather than an error. `Err(AppError::Json)` when
/// the file exists but does not parse, `Err(AppError::Io)` if it cannot be read.
pub fn load_profiles(paths: &Paths) -> Result<Vec<Profile>> {
    let path = paths.profiles();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

/// Rewrite the profile list wholesale. Pretty-printed because the file is
/// something a user may reasonably open and edit.
///
/// `Profile` has no password field, so nothing secret can reach the disk here —
/// see [`crate::secrets`].
///
/// # Arguments
/// * `paths` — `&Paths`: roots for the config and data directories.
/// * `profiles` — `&[Profile]`: the complete list to persist, in the order it
///   should appear; an empty slice legitimately clears the file.
///
/// # Returns
/// `Result<()>` — unit on success. `Err(AppError::Json)` if serialisation
/// fails, `Err(AppError::Io)` if the write does.
pub fn save_profiles(paths: &Paths, profiles: &[Profile]) -> Result<()> {
    let text = serde_json::to_string_pretty(profiles)?;
    std::fs::write(paths.profiles(), text)?;
    Ok(())
}

/// Add a profile or replace the existing one with the same id.
///
/// The connect modal saves through this for both new and edited profiles, so
/// matching on id — not name — is what stops an edit from duplicating the entry.
///
/// # Arguments
/// * `paths` — `&Paths`: roots for the config and data directories.
/// * `profile` — `Profile`: taken by value and stored as-is; its `id` is the
///   match key, and everything else is webview-supplied connection settings.
///
/// # Returns
/// `Result<()>` — unit once the whole list has been rewritten. Propagates the
/// load and save errors, so a corrupt `profiles.json` fails here rather than
/// being overwritten.
pub fn upsert_profile(paths: &Paths, profile: Profile) -> Result<()> {
    let mut profiles = load_profiles(paths)?;
    match profiles.iter_mut().find(|p| p.id == profile.id) {
        Some(existing) => *existing = profile,
        None => profiles.push(profile),
    }
    save_profiles(paths, &profiles)
}

/// Forget a profile. Removing an id that is not there is not an error, so a
/// double-click on delete cannot fail the second time.
///
/// Only the on-disk entry: the caller is responsible for the keychain password.
///
/// # Arguments
/// * `paths` — `&Paths`: roots for the config and data directories.
/// * `id` — `&str`: webview-supplied profile id; one that matches nothing
///   leaves the list untouched.
///
/// # Returns
/// `Result<()>` — unit whether or not anything was removed. Propagates the
/// load and save errors.
pub fn remove_profile(paths: &Paths, id: &str) -> Result<()> {
    let mut profiles = load_profiles(paths)?;
    profiles.retain(|p| p.id != id);
    save_profiles(paths, &profiles)
}

// -------------------------------- history -------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryItem {
    pub input: String,
    /// Unix seconds.
    pub at: i64,
}

/// The terminal's input history, oldest first.
///
/// Deliberately lossy: a line that does not parse is dropped and the rest of the
/// file is kept. History is a convenience, and a single truncated write — the
/// app killed mid-append — must not cost the user the other 999 entries or, worse,
/// fail the `history_list` command and leave the sidebar panel broken.
///
/// # Arguments
/// * `paths` — `&Paths`: roots for the config and data directories.
///
/// # Returns
/// `Result<Vec<HistoryItem>>` — entries oldest first, at most `HISTORY_CAP` of
/// them. Empty when the file is absent or every line was unparseable.
/// `Err(AppError::Io)` only if the file exists and cannot be read.
pub fn load_history(paths: &Paths) -> Result<Vec<HistoryItem>> {
    let path = paths.history();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path)?;
    Ok(text
        .lines()
        .filter(|l| !l.trim().is_empty())
        // Skip malformed lines rather than losing the whole history.
        .filter_map(|l| serde_json::from_str::<HistoryItem>(l).ok())
        .collect())
}

/// Record one terminal input, stamped with the current time.
///
/// Blank input is silently ignored rather than rejected — the user pressing
/// enter on an empty prompt is not an error the UI should surface.
///
/// Despite the name this rewrites the whole file, because enforcing
/// `HISTORY_CAP` means dropping from the *front*: the oldest entries are
/// evicted so the file cannot grow without bound. That also means a corrupt line
/// tolerated by [`load_history`] is quietly dropped by the next append.
///
/// # Arguments
/// * `paths` — `&Paths`: roots for the config and data directories.
/// * `input` — `&str`: untrusted terminal input from the webview, stored
///   trimmed; blank or whitespace-only is a no-op rather than an error.
///
/// # Returns
/// `Result<()>` — unit once the capped file has been rewritten. Propagates the
/// load, serialisation and write errors.
pub fn append_history(paths: &Paths, input: &str) -> Result<()> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    let mut items = load_history(paths)?;
    items.push(HistoryItem {
        input: trimmed.to_string(),
        at: chrono::Utc::now().timestamp(),
    });
    if items.len() > HISTORY_CAP {
        items.drain(0..items.len() - HISTORY_CAP);
    }

    let mut out = String::new();
    for item in &items {
        out.push_str(&serde_json::to_string(item)?);
        out.push('\n');
    }
    std::fs::write(paths.history(), out)?;
    Ok(())
}

// ----------------------------- saved queries ----------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SavedQuery {
    pub name: String,
    pub path: String,
    pub content: String,
}

/// The three examples the design's sidebar shows, written on first run so the
/// panel isn't empty.
const SEED_QUERIES: &[(&str, &str)] = &[
    (
        "dau_last_30d",
        "-- Daily active users over the last 30 days\n\
         SELECT date_trunc('day', created_at) AS day,\n\
         \x20      count(DISTINCT user_id) AS dau\n\
         FROM events\n\
         WHERE created_at > now() - interval '30 days'\n\
         GROUP BY 1\n\
         ORDER BY 1 DESC;\n",
    ),
    (
        "funnel_signup_activate",
        "-- Signup -> activation funnel\n\
         WITH signups AS (\n\
         \x20 SELECT DISTINCT user_id FROM events WHERE event_name = 'signup'\n\
         ), activated AS (\n\
         \x20 SELECT DISTINCT user_id FROM events WHERE event_name = 'feature_used'\n\
         )\n\
         SELECT (SELECT count(*) FROM signups)   AS signups,\n\
         \x20      (SELECT count(*) FROM activated) AS activated;\n",
    ),
    (
        "top_events_hourly",
        "-- Top events by hour\n\
         SELECT date_trunc('hour', created_at) AS hour,\n\
         \x20      event_name,\n\
         \x20      count(*) AS n\n\
         FROM events\n\
         WHERE created_at > now() - interval '24 hours'\n\
         GROUP BY 1, 2\n\
         ORDER BY 1 DESC, 3 DESC;\n",
    ),
];

/// Write the `SEED_QUERIES` examples on first run.
///
/// Runs on every startup, but only writes a file that is not already there, so
/// an edited seed keeps the user's version and a deleted one stays deleted only
/// until the next launch.
pub fn seed_saved_queries(paths: &Paths) -> Result<()> {
    let dir = paths.saved_queries();
    for (name, content) in SEED_QUERIES {
        let path = dir.join(format!("{name}.sql"));
        if !path.exists() {
            std::fs::write(path, content)?;
        }
    }
    Ok(())
}

/// How deep the saved-queries tree may nest.
///
/// Not a security boundary — the containment check is that — but a guard
/// against a pathological name turning the sidebar into an unusable staircase.
const MAX_FOLDER_DEPTH: usize = 4;

/// One path segment reduced to characters that are safe in a filename.
fn sanitize_segment(segment: &str) -> String {
    segment
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Turn a user-supplied name into a relative path under the saved-queries
/// directory, with folders written as `reports/daily`.
///
/// This is the *only* place a name from the webview becomes a path, and it is
/// belt-and-braces with the containment checks below: `..` and absolute paths
/// cannot survive segment sanitisation, and the containment check would catch
/// them anyway if they did.
pub fn sanitize_relative(name: &str) -> Result<PathBuf> {
    let segments: Vec<String> = name
        .split(['/', '\\'])
        .map(sanitize_segment)
        .filter(|s| !s.is_empty())
        .collect();

    if segments.is_empty() {
        return Err(AppError::Invalid("query name is empty".into()));
    }
    if segments.len() > MAX_FOLDER_DEPTH {
        return Err(AppError::Invalid(format!(
            "too many folders — at most {} levels",
            MAX_FOLDER_DEPTH - 1
        )));
    }
    Ok(segments.iter().collect())
}

/// Resolve a webview-supplied path to an existing file inside the saved-queries
/// directory.
///
/// The path is never trusted. It must canonicalise to a `.sql` file *under* the
/// canonical saved-queries directory — `starts_with` on `Path` compares whole
/// components, so a sibling directory like `saved_queries_evil` cannot pass by
/// sharing a string prefix. Without this, every command taking a path would be
/// an arbitrary-file primitive reachable from the frontend.
fn resolve_existing(paths: &Paths, path: &str) -> Result<PathBuf> {
    let real_dir = paths
        .saved_queries()
        .canonicalize()
        .map_err(|e| AppError::Storage(format!("saved-queries directory: {e}")))?;
    let real_path = std::path::Path::new(path)
        .canonicalize()
        .map_err(|_| AppError::Invalid(format!("no such saved query: {path}")))?;

    if !real_path.starts_with(&real_dir) || real_path == real_dir {
        return Err(AppError::Invalid(
            "refusing to touch anything outside the saved-queries directory".into(),
        ));
    }
    Ok(real_path)
}

/// Resolve a relative name to a path for a file that does not exist yet.
///
/// The target cannot be canonicalised, so its *parent* is: any symlink or `..`
/// in the folder part resolves before the containment check, and the file name
/// itself has already been through [`sanitize_relative`].
fn resolve_new(paths: &Paths, relative: &std::path::Path) -> Result<PathBuf> {
    let dir = paths.saved_queries();
    let target = dir.join(relative).with_extension("sql");

    let parent = target
        .parent()
        .ok_or_else(|| AppError::Invalid("invalid query name".into()))?;
    std::fs::create_dir_all(parent)?;

    let real_dir = dir
        .canonicalize()
        .map_err(|e| AppError::Storage(format!("saved-queries directory: {e}")))?;
    let real_parent = parent
        .canonicalize()
        .map_err(|e| AppError::Storage(format!("saved-queries folder: {e}")))?;
    if !real_parent.starts_with(&real_dir) {
        return Err(AppError::Invalid(
            "refusing to write outside the saved-queries directory".into(),
        ));
    }

    Ok(real_parent.join(target.file_name().unwrap_or_default()))
}

/// The display name for a saved query: its path under the saved-queries
/// directory, without the extension, using `/` on every platform so the
/// frontend can split it into folders without knowing the host separator.
fn relative_name(dir: &std::path::Path, path: &std::path::Path) -> String {
    relative_path(dir, &path.with_extension(""))
}

/// `path` expressed relative to `dir`, with `/` separators. Split out from
/// [`relative_name`] because folders keep their name as-is — stripping an
/// "extension" from a folder called `v1.2` would silently rename it.
fn relative_path(dir: &std::path::Path, path: &std::path::Path) -> String {
    let stem = path.to_path_buf();
    // Paths that have been through `canonicalize` differ from the configured
    // directory on macOS (`/var` is a symlink to `/private/var`), so try the
    // resolved form too — otherwise the name comes back as an absolute path.
    let canonical = dir.canonicalize();
    let relative = stem
        .strip_prefix(dir)
        .or_else(|_| stem.strip_prefix(canonical.as_deref().unwrap_or(dir)))
        .unwrap_or(&stem);
    relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Recursively gather the `.sql` files under `dir`, appending to `out`.
///
/// `root` is what names are reported relative to and stays fixed across the
/// recursion, so a subtree can be collected on its own — [`rename_saved_folder`]
/// does exactly that — while still producing full names like `reports/dau`.
///
/// A file that cannot be read contributes an empty `content` rather than failing
/// the whole listing, on the same principle as [`load_history`]: one bad file
/// must not empty the sidebar.
fn collect_saved_queries(
    root: &std::path::Path,
    dir: &std::path::Path,
    depth: usize,
    out: &mut Vec<SavedQuery>,
) -> Result<()> {
    if depth > MAX_FOLDER_DEPTH {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        // `file_type` rather than `metadata`, so a symlink pointing outside the
        // directory is skipped instead of followed.
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_saved_queries(root, &path, depth + 1, out)?;
        } else if file_type.is_file() && path.extension().and_then(|e| e.to_str()) == Some("sql") {
            out.push(SavedQuery {
                name: relative_name(root, &path),
                path: path.to_string_lossy().into_owned(),
                content: std::fs::read_to_string(&path).unwrap_or_default(),
            });
        }
    }
    Ok(())
}

/// Every saved query, contents included, sorted so the sidebar can build its
/// folder tree in a single pass.
pub fn load_saved_queries(paths: &Paths) -> Result<Vec<SavedQuery>> {
    let dir = paths.saved_queries();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    collect_saved_queries(&dir, &dir, 0, &mut out)?;
    // Sorting by name puts each folder's contents together, since the name is
    // the folder path — the sidebar can then build its tree in one pass.
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Save a query under a webview-supplied *name*, creating any folders it names.
///
/// The counterpart to [`save_query_at`], which addresses an existing file by
/// path; this one is for "Save as", so the name goes through
/// [`sanitize_relative`] and the returned `name`/`path` are the sanitised ones —
/// they may differ from what was asked for, and the caller should adopt them.
///
/// Overwrites silently if the name already exists.
pub fn save_query(paths: &Paths, name: &str, content: &str, overwrite: bool) -> Result<SavedQuery> {
    let relative = sanitize_relative(name)?;
    let path = resolve_new(paths, &relative)?;
    // Saving under a name already in use would destroy that query's contents
    // with no prompt, so the caller has to opt in — the same rule
    // `rename_saved_query` already enforces. Save-in-place is `save_query_at`,
    // which is explicitly the overwrite path.
    if !overwrite && path.exists() {
        return Err(AppError::Exists(format!(
            "a saved query named {name:?} already exists"
        )));
    }
    std::fs::write(&path, content)?;
    Ok(SavedQuery {
        name: relative_name(&paths.saved_queries(), &path),
        path: path.to_string_lossy().into_owned(),
        content: content.to_string(),
    })
}

/// Overwrite an existing saved query, identified by path.
pub fn save_query_at(paths: &Paths, path: &str, content: &str) -> Result<SavedQuery> {
    let real_path = resolve_existing(paths, path)?;
    if real_path.extension().and_then(|e| e.to_str()) != Some("sql") {
        return Err(AppError::Invalid("saved queries must be .sql files".into()));
    }

    std::fs::write(&real_path, content)?;
    Ok(SavedQuery {
        name: relative_name(&paths.saved_queries(), &real_path),
        path: real_path.to_string_lossy().into_owned(),
        content: content.to_string(),
    })
}

/// Rename or move a saved query. `new_name` may contain `/` to move it into a
/// folder, which is also how the sidebar's drag-between-folders works.
pub fn rename_saved_query(paths: &Paths, path: &str, new_name: &str) -> Result<SavedQuery> {
    let source = resolve_existing(paths, path)?;
    if !source.is_file() {
        return Err(AppError::Invalid("not a saved query".into()));
    }
    let relative = sanitize_relative(new_name)?;
    let target = resolve_new(paths, &relative)?;

    if target == source {
        // A no-op rename, not an error — the name sanitised to what it already
        // was (`My Query` and `My Query` differing only in a stripped char).
        let content = std::fs::read_to_string(&source).unwrap_or_default();
        return Ok(SavedQuery {
            name: relative_name(&paths.saved_queries(), &source),
            path: source.to_string_lossy().into_owned(),
            content,
        });
    }
    if target.exists() {
        // Renaming onto an existing query would destroy it silently, and the
        // user cannot see the collision if it is in another folder.
        return Err(AppError::Invalid(format!(
            "a saved query named {new_name:?} already exists"
        )));
    }

    std::fs::rename(&source, &target)?;
    Ok(SavedQuery {
        name: relative_name(&paths.saved_queries(), &target),
        path: target.to_string_lossy().into_owned(),
        content: std::fs::read_to_string(&target).unwrap_or_default(),
    })
}

/// Delete a saved query.
pub fn delete_saved_query(paths: &Paths, path: &str) -> Result<()> {
    let real_path = resolve_existing(paths, path)?;
    if real_path.extension().and_then(|e| e.to_str()) != Some("sql") {
        return Err(AppError::Invalid("saved queries must be .sql files".into()));
    }
    std::fs::remove_file(&real_path)?;

    // Drop the folder if that was its last query, so deleting the contents of a
    // folder doesn't leave an empty one behind with no way to remove it.
    if let Some(parent) = real_path.parent() {
        let root = paths.saved_queries().canonicalize().ok();
        let is_empty = std::fs::read_dir(parent).is_ok_and(|mut d| d.next().is_none());
        if Some(parent.to_path_buf()) != root && is_empty {
            let _ = std::fs::remove_dir(parent);
        }
    }
    Ok(())
}

/// Every folder under the saved-queries directory, including empty ones.
///
/// Listed separately from the queries because a folder with nothing in it
/// cannot be inferred from the file list — and a "New folder" that vanishes
/// until something is moved into it is not a folder the user can use.
pub fn list_saved_folders(paths: &Paths) -> Result<Vec<String>> {
    let root = paths.saved_queries();
    if !root.exists() {
        return Ok(Vec::new());
    }

    /// Directories only, depth-first, each pushed as a `/`-joined name. Nested
    /// inside its caller because collecting folder names is useful nowhere else.
    fn walk(
        root: &std::path::Path,
        dir: &std::path::Path,
        depth: usize,
        out: &mut Vec<String>,
    ) -> Result<()> {
        if depth > MAX_FOLDER_DEPTH {
            return Ok(());
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let path = entry.path();
                out.push(relative_path(root, &path));
                walk(root, &path, depth + 1, out)?;
            }
        }
        Ok(())
    }

    let mut out = Vec::new();
    walk(&root, &root, 0, &mut out)?;
    out.sort();
    Ok(out)
}

/// One query's path before and after a folder rename, so open editor tabs can
/// follow the files they came from.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovedQuery {
    pub from: String,
    pub query: SavedQuery,
}

/// Rename a folder, moving everything inside it.
///
/// A real directory rename rather than moving each query in turn: it is atomic,
/// it is one filesystem call instead of N, and it works on an empty folder —
/// which the per-query version silently would not.
pub fn rename_saved_folder(paths: &Paths, path: &str, new_name: &str) -> Result<Vec<MovedQuery>> {
    let root = paths.saved_queries();
    let source = resolve_existing(paths, &root.join(path).to_string_lossy())?;
    if !source.is_dir() {
        return Err(AppError::Invalid("not a folder".into()));
    }

    // The new name replaces the last segment only; the folder stays where it is.
    let parent_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut relative = sanitize_relative(new_name)?;
    if parent_segments.len() > 1 {
        let parent: PathBuf = parent_segments[..parent_segments.len() - 1]
            .iter()
            .collect();
        relative = parent.join(relative);
    }

    let target = root.join(&relative);
    if target == source {
        return Ok(Vec::new());
    }
    if target.exists() {
        return Err(AppError::Invalid(format!(
            "a folder named {new_name:?} already exists"
        )));
    }
    // Moving a folder into itself would orphan everything under it.
    if target.starts_with(&source) {
        return Err(AppError::Invalid(
            "cannot move a folder inside itself".into(),
        ));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Record what is inside before the move, so the paths can be paired up.
    let mut before = Vec::new();
    collect_saved_queries(&root, &source, 0, &mut before)?;

    std::fs::rename(&source, &target)?;

    let mut after = Vec::new();
    collect_saved_queries(&root, &target, 0, &mut after)?;
    after.sort_by(|a, b| a.name.cmp(&b.name));
    before.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(before
        .into_iter()
        .zip(after)
        .map(|(from, query)| MovedQuery {
            from: from.path,
            query,
        })
        .collect())
}

/// Create an empty folder.
pub fn create_saved_folder(paths: &Paths, name: &str) -> Result<String> {
    let relative = sanitize_relative(name)?;
    let dir = paths.saved_queries().join(&relative);
    // Reuse the containment check by resolving a would-be file inside it.
    resolve_new(paths, &relative.join("placeholder"))?;
    std::fs::create_dir_all(&dir)?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

// ------------------------------ ER layout -------------------------------

/// Dragged card positions, keyed `schema` -> `table` -> (x, y).
pub type ErLayout = std::collections::HashMap<String, std::collections::HashMap<String, [f64; 2]>>;

/// Saved card positions, empty when nothing has been dragged yet.
///
/// A corrupt file degrades to an empty layout rather than erroring: positions
/// are cosmetic, and the diagram re-laying itself out automatically is a far
/// better failure than the Relationships tab refusing to open at all.
pub fn load_er_layout(paths: &Paths) -> Result<ErLayout> {
    let path = paths.er_layout();
    if !path.exists() {
        return Ok(ErLayout::new());
    }
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

/// Persist card positions, replacing the file wholesale — the frontend owns the
/// whole layout and sends it back in full after a drag settles.
pub fn save_er_layout(paths: &Paths, layout: &ErLayout) -> Result<()> {
    std::fs::write(paths.er_layout(), serde_json::to_string_pretty(layout)?)?;
    Ok(())
}

// ------------------------------ grid layout -----------------------------

/// One table's grid column layout.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableLayout {
    /// Column name -> pixel width. Absent means "use the computed default".
    #[serde(default)]
    pub widths: std::collections::HashMap<String, f64>,
    /// Display order by column name. May be stale relative to the real table;
    /// reconciling that is the frontend's job.
    #[serde(default)]
    pub order: Vec<String>,
}

/// Keyed "schema.table".
pub type GridLayout = std::collections::HashMap<String, TableLayout>;

/// Narrow enough to still show a drag handle, wide enough that the *next*
/// column's handle stays reachable on a small window. A persisted 0 or a
/// negative width would collapse the column to nothing, and the user could then
/// never grab its handle to undo it — the setting would be unfixable from the UI.
const MIN_COLUMN_WIDTH: f64 = 32.0;
const MAX_COLUMN_WIDTH: f64 = 2000.0;

/// How many tables' layouts to keep.
///
/// Browsing a large database touches hundreds of tables, and every one visited
/// would otherwise be remembered forever. A few hundred entries is far more than
/// anyone has open tabs for, while keeping the file trivially small.
const GRID_LAYOUT_CAP: usize = 200;

/// Every table's saved column layout. An absent entry means "no saved layout";
/// the frontend then computes defaults from the data.
pub fn load_grid_layout(paths: &Paths) -> Result<GridLayout> {
    let path = paths.grid_layout();
    if !path.exists() {
        return Ok(GridLayout::new());
    }
    // A layout is a convenience; a corrupt one must never stop the user from
    // browsing data, so it degrades to "no saved layout" rather than an error.
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(GridLayout::new());
    };
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

/// Merge one table's layout into the stored map and rewrite the file.
pub fn save_table_layout(paths: &Paths, key: &str, layout: &TableLayout) -> Result<()> {
    let sanitized = TableLayout {
        // Non-finite widths come from a division by zero somewhere upstream and
        // are not representable in JSON anyway — drop them so the column falls
        // back to its computed default instead of persisting a poison value.
        widths: layout
            .widths
            .iter()
            .filter(|(_, w)| w.is_finite())
            .map(|(c, w)| (c.clone(), w.clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH)))
            .collect(),
        order: layout.order.clone(),
    };

    let mut all = load_grid_layout(paths)?;
    all.insert(key.to_string(), sanitized);

    // No recency information is stored on disk (the wire shape is the layout
    // itself), so evict in reverse key order: arbitrary, but deterministic and
    // never the table just saved.
    while all.len() > GRID_LAYOUT_CAP {
        let Some(victim) = all.keys().filter(|k| k.as_str() != key).max().cloned() else {
            break;
        };
        all.remove(&victim);
    }

    std::fs::write(paths.grid_layout(), serde_json::to_string_pretty(&all)?)?;
    Ok(())
}

/// Test helper: a `Paths` rooted at a temporary directory.
///
/// Also compiled for the `integration` feature, since the end-to-end terminal
/// tests need a saved-queries directory of their own — `\i` must never be
/// tested against the developer's real one.
#[cfg(any(test, feature = "integration"))]
pub fn paths_in(dir: &std::path::Path) -> Paths {
    let p = Paths {
        config_dir: dir.join("config"),
        data_dir: dir.join("data"),
    };
    std::fs::create_dir_all(&p.config_dir).unwrap();
    std::fs::create_dir_all(p.data_dir.join("saved_queries")).unwrap();
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect::SslMode;

    /// A fresh, uniquely named directory to build `Paths` from, so tests never
    /// touch the real store and never collide when run in parallel.
    fn tmpdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pgscope-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A complete profile whose only varying field is the id — these tests are
    /// about persistence, not about the connection settings themselves.
    fn profile(id: &str) -> Profile {
        Profile {
            id: id.into(),
            name: format!("profile {id}"),
            host: "localhost".into(),
            port: 5432,
            database: "analytics_prod".into(),
            user: "pgscope".into(),
            sslmode: SslMode::Prefer,
        }
    }

    #[test]
    fn profiles_round_trip() {
        let dir = tmpdir();
        let paths = paths_in(&dir);

        assert!(load_profiles(&paths).unwrap().is_empty());

        upsert_profile(&paths, profile("a")).unwrap();
        upsert_profile(&paths, profile("b")).unwrap();
        assert_eq!(load_profiles(&paths).unwrap().len(), 2);

        // Upserting an existing id replaces rather than duplicates.
        let mut updated = profile("a");
        updated.name = "renamed".into();
        upsert_profile(&paths, updated).unwrap();
        let profiles = load_profiles(&paths).unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(
            profiles.iter().find(|p| p.id == "a").unwrap().name,
            "renamed"
        );

        remove_profile(&paths, "a").unwrap();
        assert_eq!(load_profiles(&paths).unwrap().len(), 1);
    }

    #[test]
    fn profiles_never_serialize_a_password_field() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        upsert_profile(&paths, profile("a")).unwrap();
        let text = std::fs::read_to_string(paths.profiles()).unwrap();
        assert!(!text.to_lowercase().contains("password"));
        assert!(!text.contains("secret"));
    }

    #[test]
    fn history_appends_and_reloads() {
        let dir = tmpdir();
        let paths = paths_in(&dir);

        append_history(&paths, "SELECT 1;").unwrap();
        append_history(&paths, "\\d events").unwrap();
        let items = load_history(&paths).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].input, "\\d events");
        assert!(items[0].at > 0);
    }

    #[test]
    fn history_ignores_blank_input() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        append_history(&paths, "   \n ").unwrap();
        assert!(load_history(&paths).unwrap().is_empty());
    }

    #[test]
    fn history_is_capped() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        for i in 0..(HISTORY_CAP + 50) {
            append_history(&paths, &format!("SELECT {i};")).unwrap();
        }
        let items = load_history(&paths).unwrap();
        assert_eq!(items.len(), HISTORY_CAP);
        // The oldest entries were dropped, newest kept.
        assert_eq!(
            items.last().unwrap().input,
            format!("SELECT {};", HISTORY_CAP + 49)
        );
    }

    #[test]
    fn history_survives_a_corrupt_line() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        append_history(&paths, "SELECT 1;").unwrap();
        let mut text = std::fs::read_to_string(paths.history()).unwrap();
        text.push_str("{not json\n");
        std::fs::write(paths.history(), text).unwrap();

        let items = load_history(&paths).unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn saved_queries_seed_and_load() {
        let dir = tmpdir();
        let paths = paths_in(&dir);

        seed_saved_queries(&paths).unwrap();
        let queries = load_saved_queries(&paths).unwrap();
        let names: Vec<&str> = queries.iter().map(|q| q.name.as_str()).collect();
        // The three the design's sidebar lists.
        assert_eq!(
            names,
            vec![
                "dau_last_30d",
                "funnel_signup_activate",
                "top_events_hourly"
            ]
        );
        assert!(queries[0].content.contains("SELECT"));
    }

    #[test]
    fn seeding_does_not_clobber_edits() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        seed_saved_queries(&paths).unwrap();

        let path = paths.saved_queries().join("dau_last_30d.sql");
        std::fs::write(&path, "-- my edit\n").unwrap();
        seed_saved_queries(&paths).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "-- my edit\n");
    }

    #[test]
    fn save_query_sanitizes_path_traversal() {
        let dir = tmpdir();
        let paths = paths_in(&dir);

        // Slashes are folder separators now, so this lands in nested folders
        // named `__` rather than being flattened to one name. What has to hold
        // is unchanged: no `..` survives, and the file stays inside.
        let saved = save_query(&paths, "../../etc/passwd", "SELECT 1;", false).unwrap();
        assert_eq!(saved.name, "__/__/etc/passwd");
        assert!(is_inside_saved_queries(&paths, &saved.path));
    }

    /// Containment check for tests, comparing resolved paths — on macOS the
    /// configured directory and a canonicalised one differ by `/private`.
    fn is_inside_saved_queries(paths: &Paths, path: &str) -> bool {
        let root = paths.saved_queries().canonicalize().unwrap();
        std::path::Path::new(path)
            .canonicalize()
            .map(|p| p.starts_with(&root))
            .unwrap_or(false)
    }

    #[test]
    fn save_query_at_overwrites_in_place() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        seed_saved_queries(&paths).unwrap();

        let target = paths.saved_queries().join("dau_last_30d.sql");
        let saved = save_query_at(&paths, target.to_str().unwrap(), "SELECT 1;\n").unwrap();

        assert_eq!(saved.name, "dau_last_30d");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "SELECT 1;\n");
        // Still exactly one file — saving in place must not create a duplicate.
        assert_eq!(load_saved_queries(&paths).unwrap().len(), 3);
    }

    #[test]
    fn save_query_at_refuses_a_path_outside_the_directory() {
        let dir = tmpdir();
        let paths = paths_in(&dir);

        // A real file that exists, but not in saved_queries.
        let outside = dir.join("victim.sql");
        std::fs::write(&outside, "original\n").unwrap();

        let err = save_query_at(&paths, outside.to_str().unwrap(), "overwritten\n").unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)), "got {err:?}");
        // The file must be untouched.
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "original\n");
    }

    #[test]
    fn save_query_at_refuses_traversal_through_the_directory() {
        let dir = tmpdir();
        let paths = paths_in(&dir);

        let outside = dir.join("victim.sql");
        std::fs::write(&outside, "original\n").unwrap();

        // A path that starts inside saved_queries but climbs back out.
        let traversal = paths
            .saved_queries()
            .join("..")
            .join("..")
            .join("victim.sql");
        let err = save_query_at(&paths, traversal.to_str().unwrap(), "overwritten\n").unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)), "got {err:?}");
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "original\n");
    }

    #[test]
    fn save_query_at_accepts_a_query_inside_a_folder() {
        // Folders are supported, so a nested path is legitimate. What still has
        // to hold is that it stays *under* the directory — see the escape tests.
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let nested = paths.saved_queries().join("reports");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("q.sql");
        std::fs::write(&file, "original\n").unwrap();

        let saved = save_query_at(&paths, file.to_str().unwrap(), "x").unwrap();
        assert_eq!(saved.name, "reports/q");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "x");
    }

    #[test]
    fn a_sibling_directory_sharing_a_name_prefix_is_still_outside() {
        // The containment check compares whole path components. A string-prefix
        // check would let `saved_queries_evil` pass as "inside saved_queries".
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let evil = paths.saved_queries().with_file_name("saved_queries_evil");
        std::fs::create_dir_all(&evil).unwrap();
        let file = evil.join("q.sql");
        std::fs::write(&file, "original\n").unwrap();

        let err = save_query_at(&paths, file.to_str().unwrap(), "x").unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)), "{err:?}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original\n");
    }

    // ------------------------ rename / delete -------------------------

    #[test]
    fn save_refuses_to_silently_overwrite_an_existing_query() {
        // Saving under a name already in use used to destroy that query with no
        // prompt — the same clobber `rename_saved_query` has always refused.
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let first = save_query(&paths, "report", "SELECT 'original';\n", false).unwrap();

        let err = save_query(&paths, "report", "SELECT 'clobber';\n", false).unwrap_err();
        assert!(matches!(err, AppError::Exists(_)), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(&first.path).unwrap(),
            "SELECT 'original';\n",
            "the original content must survive"
        );
    }

    #[test]
    fn save_overwrites_when_the_caller_opts_in() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let first = save_query(&paths, "report", "SELECT 'original';\n", false).unwrap();

        save_query(&paths, "report", "SELECT 'replaced';\n", true).unwrap();
        assert_eq!(
            std::fs::read_to_string(&first.path).unwrap(),
            "SELECT 'replaced';\n"
        );
        assert_eq!(load_saved_queries(&paths).unwrap().len(), 1);
    }

    #[test]
    fn the_collision_check_is_per_folder() {
        // `reports/a` and `a` are different queries; sharing a leaf name is not
        // a collision.
        let dir = tmpdir();
        let paths = paths_in(&dir);
        save_query(&paths, "a", "SELECT 1;\n", false).unwrap();
        save_query(&paths, "reports/a", "SELECT 2;\n", false).unwrap();
        assert_eq!(load_saved_queries(&paths).unwrap().len(), 2);
    }

    #[test]
    fn rename_moves_a_query_and_reports_its_new_name() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let original = save_query(&paths, "old_name", "SELECT 1;\n", false).unwrap();

        let renamed = rename_saved_query(&paths, &original.path, "new_name").unwrap();
        assert_eq!(renamed.name, "new_name");
        assert_eq!(renamed.content, "SELECT 1;\n");
        assert!(!std::path::Path::new(&original.path).exists());
    }

    #[test]
    fn rename_can_move_a_query_into_a_folder() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let original = save_query(&paths, "dau", "SELECT 1;\n", false).unwrap();

        let moved = rename_saved_query(&paths, &original.path, "reports/dau").unwrap();
        assert_eq!(moved.name, "reports/dau");
        assert_eq!(load_saved_queries(&paths).unwrap().len(), 1);
    }

    #[test]
    fn rename_refuses_to_clobber_an_existing_query() {
        // Silently overwriting would destroy work, and the collision may be in
        // a folder the user cannot currently see.
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let a = save_query(&paths, "a", "SELECT 'a';\n", false).unwrap();
        save_query(&paths, "b", "SELECT 'b';\n", false).unwrap();

        let err = rename_saved_query(&paths, &a.path, "b").unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
        // Both must survive intact.
        assert_eq!(load_saved_queries(&paths).unwrap().len(), 2);
        assert_eq!(std::fs::read_to_string(&a.path).unwrap(), "SELECT 'a';\n");
    }

    #[test]
    fn renaming_a_query_to_its_own_name_is_not_an_error() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let a = save_query(&paths, "a", "SELECT 1;\n", false).unwrap();

        let same = rename_saved_query(&paths, &a.path, "a").unwrap();
        assert_eq!(same.name, "a");
        assert!(std::path::Path::new(&a.path).exists());
    }

    #[test]
    fn rename_sanitizes_traversal_out_of_the_directory() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let a = save_query(&paths, "a", "SELECT 1;\n", false).unwrap();

        let renamed = rename_saved_query(&paths, &a.path, "../../escaped").unwrap();
        // `..` segments sanitise to `__`, so the file stays put rather than
        // landing two directories up.
        assert!(!renamed.name.contains(".."), "{}", renamed.name);
        assert!(is_inside_saved_queries(&paths, &renamed.path));
    }

    #[test]
    fn delete_removes_the_file() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let a = save_query(&paths, "doomed", "SELECT 1;\n", false).unwrap();

        delete_saved_query(&paths, &a.path).unwrap();
        assert!(!std::path::Path::new(&a.path).exists());
        assert!(load_saved_queries(&paths).unwrap().is_empty());
    }

    #[test]
    fn delete_refuses_a_path_outside_the_directory() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let outside = dir.join("important.sql");
        std::fs::write(&outside, "keep me\n").unwrap();

        let err = delete_saved_query(&paths, outside.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)), "{err:?}");
        assert!(outside.exists(), "the file must still be there");
    }

    #[test]
    fn delete_refuses_a_non_sql_file() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let notes = paths.saved_queries().join("notes.txt");
        std::fs::write(&notes, "keep me\n").unwrap();

        let err = delete_saved_query(&paths, notes.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
        assert!(notes.exists());
    }

    #[test]
    fn deleting_the_last_query_in_a_folder_removes_the_folder() {
        // Otherwise an empty folder lingers with nothing in the UI to remove it.
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let a = save_query(&paths, "reports/only", "SELECT 1;\n", false).unwrap();
        let folder = paths.saved_queries().join("reports");
        assert!(folder.exists());

        delete_saved_query(&paths, &a.path).unwrap();
        assert!(!folder.exists());
    }

    #[test]
    fn deleting_one_of_several_leaves_the_folder_alone() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let a = save_query(&paths, "reports/a", "SELECT 1;\n", false).unwrap();
        save_query(&paths, "reports/b", "SELECT 2;\n", false).unwrap();

        delete_saved_query(&paths, &a.path).unwrap();
        assert!(paths.saved_queries().join("reports").exists());
    }

    // ---------------------------- folders -----------------------------

    #[test]
    fn queries_in_folders_are_named_by_their_relative_path() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        save_query(&paths, "top", "SELECT 1;\n", false).unwrap();
        save_query(&paths, "reports/dau", "SELECT 2;\n", false).unwrap();
        save_query(&paths, "reports/deep/wau", "SELECT 3;\n", false).unwrap();

        let names: Vec<String> = load_saved_queries(&paths)
            .unwrap()
            .into_iter()
            .map(|q| q.name)
            .collect();
        // Sorted by name, which groups each folder's contents together — the
        // property the sidebar's one-pass tree build relies on.
        assert_eq!(names, vec!["reports/dau", "reports/deep/wau", "top"]);
    }

    #[test]
    fn folder_names_always_use_forward_slashes() {
        // The frontend splits on `/` and must not have to know the host
        // separator; on Windows the path would otherwise come back with `\`.
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let q = save_query(&paths, "reports/dau", "SELECT 1;\n", false).unwrap();
        assert_eq!(q.name, "reports/dau");
        assert!(!q.name.contains('\\'));
    }

    #[test]
    fn nesting_is_capped() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let deep = "a/b/c/d/e/f/g";
        assert!(sanitize_relative(deep).is_err());
        assert!(save_query(&paths, deep, "SELECT 1;\n", false).is_err());
    }

    #[test]
    fn sanitize_relative_strips_empty_and_dangerous_segments() {
        assert_eq!(
            sanitize_relative("reports//dau").unwrap(),
            std::path::Path::new("reports").join("dau")
        );
        assert_eq!(
            sanitize_relative("/leading").unwrap(),
            std::path::Path::new("leading")
        );
        // `..` becomes `__`: a folder with a silly name, not a way upward.
        let up = sanitize_relative("../x").unwrap();
        assert!(!up.to_string_lossy().contains(".."), "{up:?}");
        assert!(sanitize_relative("   ").is_err());
        assert!(sanitize_relative("").is_err());
    }

    #[test]
    fn renaming_a_folder_moves_everything_inside_it() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let a = save_query(&paths, "reports/dau", "SELECT 1;\n", false).unwrap();
        save_query(&paths, "reports/deep/wau", "SELECT 2;\n", false).unwrap();

        let moved = rename_saved_folder(&paths, "reports", "analytics").unwrap();
        assert_eq!(moved.len(), 2);

        let names: Vec<String> = load_saved_queries(&paths)
            .unwrap()
            .into_iter()
            .map(|q| q.name)
            .collect();
        assert_eq!(names, vec!["analytics/dau", "analytics/deep/wau"]);
        assert!(!std::path::Path::new(&a.path).exists());
    }

    #[test]
    fn a_folder_rename_pairs_each_old_path_with_its_new_one() {
        // Editor tabs are retargeted from these pairs, so a mismatched pairing
        // would silently point a tab at a different query's file.
        let dir = tmpdir();
        let paths = paths_in(&dir);
        save_query(&paths, "reports/a", "SELECT 'a';\n", false).unwrap();
        save_query(&paths, "reports/b", "SELECT 'b';\n", false).unwrap();

        let moves = rename_saved_folder(&paths, "reports", "archive").unwrap();
        assert_eq!(moves.len(), 2);
        for m in moves {
            let old_leaf = std::path::Path::new(&m.from).file_name().unwrap();
            let new_leaf = std::path::Path::new(&m.query.path).file_name().unwrap();
            assert_eq!(old_leaf, new_leaf, "paired the wrong files");
            // And the content travelled with the name.
            let leaf = m.query.name.trim_start_matches("archive/");
            assert_eq!(m.query.content, format!("SELECT '{leaf}';\n"));
        }
    }

    #[test]
    fn an_empty_folder_can_still_be_renamed() {
        // Moving each query in turn would silently do nothing here.
        let dir = tmpdir();
        let paths = paths_in(&dir);
        create_saved_folder(&paths, "scratch").unwrap();

        let moved = rename_saved_folder(&paths, "scratch", "drafts").unwrap();
        assert!(moved.is_empty());
        assert_eq!(list_saved_folders(&paths).unwrap(), vec!["drafts"]);
    }

    #[test]
    fn renaming_a_nested_folder_keeps_it_in_place() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        save_query(&paths, "a/b/q", "SELECT 1;\n", false).unwrap();

        rename_saved_folder(&paths, "a/b", "c").unwrap();
        // `a/c`, not a new top-level `c`: the new name replaces the last
        // segment only.
        assert_eq!(load_saved_queries(&paths).unwrap()[0].name, "a/c/q");
    }

    #[test]
    fn a_folder_rename_refuses_to_clobber_another_folder() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        save_query(&paths, "a/q", "SELECT 1;\n", false).unwrap();
        save_query(&paths, "b/q", "SELECT 2;\n", false).unwrap();

        let err = rename_saved_folder(&paths, "a", "b").unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
        assert_eq!(load_saved_queries(&paths).unwrap().len(), 2);
    }

    #[test]
    fn a_folder_rename_refuses_a_path_outside_the_directory() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let outside = dir.join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();

        let err = rename_saved_folder(&paths, "../elsewhere", "moved").unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)), "{err:?}");
        assert!(outside.exists(), "the folder must still be there");
    }

    #[test]
    fn empty_folders_are_listed_so_the_sidebar_can_show_them() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        create_saved_folder(&paths, "empty").unwrap();
        save_query(&paths, "used/q", "SELECT 1;\n", false).unwrap();

        assert_eq!(list_saved_folders(&paths).unwrap(), vec!["empty", "used"]);
    }

    #[test]
    fn nested_folders_are_listed_by_full_path() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        save_query(&paths, "a/b/q", "SELECT 1;\n", false).unwrap();
        assert_eq!(list_saved_folders(&paths).unwrap(), vec!["a", "a/b"]);
    }

    #[test]
    fn create_folder_returns_a_slash_separated_name() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let name = create_saved_folder(&paths, "reports").unwrap();
        assert_eq!(name, "reports");
        assert!(paths.saved_queries().join("reports").is_dir());
    }

    #[test]
    fn save_query_at_refuses_a_non_sql_file() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let other = paths.saved_queries().join("notes.txt");
        std::fs::write(&other, "original\n").unwrap();

        let err = save_query_at(&paths, other.to_str().unwrap(), "x").unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "original\n");
    }

    #[test]
    fn save_query_at_reports_a_missing_file_clearly() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        let missing = paths.saved_queries().join("nope.sql");

        let err = save_query_at(&paths, missing.to_str().unwrap(), "x").unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[test]
    fn er_layout_round_trips() {
        let dir = tmpdir();
        let paths = paths_in(&dir);

        let mut layout = ErLayout::new();
        let mut public = std::collections::HashMap::new();
        public.insert("events".to_string(), [726.0, 140.0]);
        layout.insert("public".to_string(), public);

        save_er_layout(&paths, &layout).unwrap();
        let loaded = load_er_layout(&paths).unwrap();
        assert_eq!(loaded["public"]["events"], [726.0, 140.0]);
    }

    #[test]
    fn er_layout_defaults_when_corrupt() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        std::fs::write(paths.er_layout(), "{{{ not json").unwrap();
        assert!(load_er_layout(&paths).unwrap().is_empty());
    }

    // -------------------------- grid layout ---------------------------

    /// A `TableLayout` from plain slices, so a round-trip case reads as one line.
    fn layout(widths: &[(&str, f64)], order: &[&str]) -> TableLayout {
        TableLayout {
            widths: widths.iter().map(|(c, w)| (c.to_string(), *w)).collect(),
            order: order.iter().map(|c| c.to_string()).collect(),
        }
    }

    #[test]
    fn grid_layout_round_trips_widths_and_order() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        assert!(load_grid_layout(&paths).unwrap().is_empty());

        save_table_layout(
            &paths,
            "public.events",
            &layout(&[("id", 80.0), ("user_id", 220.5)], &["user_id", "id"]),
        )
        .unwrap();

        let loaded = load_grid_layout(&paths).unwrap();
        let events = &loaded["public.events"];
        assert_eq!(events.widths["id"], 80.0);
        assert_eq!(events.widths["user_id"], 220.5);
        assert_eq!(events.order, vec!["user_id", "id"]);
    }

    #[test]
    fn saving_one_table_leaves_the_others_alone() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        save_table_layout(&paths, "public.events", &layout(&[("id", 80.0)], &["id"])).unwrap();
        save_table_layout(&paths, "public.users", &layout(&[("email", 300.0)], &[])).unwrap();

        let loaded = load_grid_layout(&paths).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded["public.events"].widths["id"], 80.0);
        assert_eq!(loaded["public.users"].widths["email"], 300.0);

        // And re-saving a table replaces its entry rather than merging into it.
        save_table_layout(&paths, "public.events", &layout(&[("ts", 120.0)], &["ts"])).unwrap();
        let loaded = load_grid_layout(&paths).unwrap();
        assert_eq!(loaded["public.events"].widths.len(), 1);
        assert_eq!(loaded["public.users"].widths["email"], 300.0);
    }

    #[test]
    fn widths_are_clamped_and_non_finite_ones_dropped() {
        // A zero or negative width leaves a column the user cannot grab again.
        let dir = tmpdir();
        let paths = paths_in(&dir);
        save_table_layout(
            &paths,
            "public.events",
            &layout(
                &[
                    ("zero", 0.0),
                    ("negative", -50.0),
                    ("huge", 1.0e9),
                    ("nan", f64::NAN),
                    ("inf", f64::INFINITY),
                    ("fine", 150.0),
                ],
                &[],
            ),
        )
        .unwrap();

        let widths = &load_grid_layout(&paths).unwrap()["public.events"].widths;
        assert_eq!(widths["zero"], MIN_COLUMN_WIDTH);
        assert_eq!(widths["negative"], MIN_COLUMN_WIDTH);
        assert_eq!(widths["huge"], MAX_COLUMN_WIDTH);
        assert_eq!(widths["fine"], 150.0);
        assert!(!widths.contains_key("nan"));
        assert!(!widths.contains_key("inf"));
    }

    #[test]
    fn grid_layout_defaults_when_corrupt() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        std::fs::write(paths.grid_layout(), "{{{ not json").unwrap();
        assert!(load_grid_layout(&paths).unwrap().is_empty());

        // And a save over the corrupt file still works, starting fresh.
        save_table_layout(&paths, "public.events", &layout(&[("id", 80.0)], &[])).unwrap();
        assert_eq!(load_grid_layout(&paths).unwrap().len(), 1);
    }

    #[test]
    fn grid_layout_is_capped_and_never_evicts_the_table_being_saved() {
        let dir = tmpdir();
        let paths = paths_in(&dir);
        for i in 0..(GRID_LAYOUT_CAP + 20) {
            // Zero-padded so key order is stable across the whole range.
            save_table_layout(
                &paths,
                &format!("public.t{i:04}"),
                &layout(&[("id", 80.0)], &[]),
            )
            .unwrap();
        }

        let loaded = load_grid_layout(&paths).unwrap();
        assert_eq!(loaded.len(), GRID_LAYOUT_CAP);
        // The most recent save survived its own eviction pass.
        let last = format!("public.t{:04}", GRID_LAYOUT_CAP + 19);
        assert!(loaded.contains_key(&last), "evicted the table just saved");
    }
}
