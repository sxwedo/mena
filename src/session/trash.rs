//! Move provider session files aside so `mena ss restore` can put them back.
//!
//! The trash directory lives next to mena’s other state (`trash/sessions`
//! under the config directory, or under a non-user home in tests). Native
//! index rows that can be reread are copied into the manifest before they
//! are removed. A failed deletion still writes the manifest for whatever
//! already moved.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::titles;

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    target: String,
    provider: String,
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    overlay_title: Option<String>,
    #[serde(default)]
    deleted_at: u64,
    moved: Vec<Moved>,
    #[serde(default)]
    cursor_rows: Vec<CursorRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Moved {
    from: PathBuf,
    stored: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CursorRow {
    pub database: PathBuf,
    pub table: String,
    pub key: String,
    pub value_hex: String,
    #[serde(default)]
    pub columns: BTreeMap<String, SqliteValue>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub updated_at: Option<i64>,
    #[serde(default)]
    pub is_archived: Option<i64>,
}

pub(super) struct TrashBatch {
    dir: PathBuf,
    provider: String,
    id: String,
    moved: Vec<Moved>,
    cursor_rows: Vec<CursorRow>,
    next: usize,
    written: bool,
    title: Option<String>,
    overlay_title: Option<String>,
    deleted_at: u64,
}

impl TrashBatch {
    pub(super) fn create(
        home: &Path,
        provider: &str,
        id: &str,
        title: Option<String>,
    ) -> Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis());
        let dir = state_dir(home)
            .join("trash/sessions")
            .join(format!("{stamp}-{provider}-{id}"));
        let overlay_title =
            titles::load(&titles::path_for(home))?.remove(&format!("{provider}:{id}"));
        if dir.exists() {
            bail!("trash entry already exists: {}", dir.display());
        }
        fs::create_dir_all(dir.join("files"))
            .with_context(|| format!("failed to create trash directory {}", dir.display()))?;
        Ok(Self {
            dir,
            provider: provider.to_owned(),
            id: id.to_owned(),
            moved: Vec::new(),
            cursor_rows: Vec::new(),
            next: 0,
            written: false,
            title,
            overlay_title,
            deleted_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |time| time.as_secs()),
        })
    }

    pub(super) fn keep_file(&mut self, path: &Path) -> Result<()> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect session artifact {}", path.display())
                });
            }
        };
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            bail!(
                "refusing to trash directory {} as a single file",
                path.display()
            );
        }
        let stored = format!("files/{:04}", self.next);
        self.next += 1;
        let destination = self.dir.join(&stored);
        move_aside(path, &destination, metadata.file_type().is_symlink()).with_context(|| {
            format!(
                "failed to move {} into trash {}",
                path.display(),
                destination.display()
            )
        })?;
        self.moved.push(Moved {
            from: path.to_path_buf(),
            stored,
        });
        Ok(())
    }

    pub(super) fn push_cursor_row(&mut self, row: CursorRow) {
        self.cursor_rows.push(row);
    }

    fn finish(mut self, permanent: bool) -> Result<()> {
        if self.moved.is_empty() && self.cursor_rows.is_empty() {
            let _ = fs::remove_dir_all(&self.dir);
            self.written = true;
            return Ok(());
        }
        self.write_manifest()?;
        if permanent {
            // Purge only this validated operation's batch after native cleanup
            // succeeds. Failures keep a recovery manifest instead of losing data.
            fs::remove_dir_all(&self.dir)
                .context("failed to permanently remove the deleted session")?;
        }
        Ok(())
    }

    fn write_manifest(&mut self) -> Result<()> {
        if self.written {
            return Ok(());
        }
        let manifest = Manifest {
            target: format!("{}:{}", self.provider, self.id),
            provider: self.provider.clone(),
            id: self.id.clone(),
            title: self.title.clone(),
            overlay_title: self.overlay_title.clone(),
            deleted_at: self.deleted_at,
            moved: self.moved.clone(),
            cursor_rows: self.cursor_rows.clone(),
        };
        let bytes =
            serde_json::to_vec_pretty(&manifest).context("failed to encode trash manifest")?;
        let path = self.dir.join("session.json");
        if !crate::fs::atomic_create_private(&path, &bytes)? {
            bail!("trash manifest already exists: {}", path.display());
        }
        self.written = true;
        Ok(())
    }
}

impl Drop for TrashBatch {
    fn drop(&mut self) {
        if self.written {
            return;
        }
        if self.moved.is_empty() && self.cursor_rows.is_empty() {
            let _ = fs::remove_dir_all(&self.dir);
            return;
        }
        let _ = self.write_manifest();
    }
}

pub(super) fn finish(batch: TrashBatch, permanent: bool) -> Result<()> {
    batch.finish(permanent)
}

/// Put one trashed session back. `target` is `provider:session-id` or the
/// trash directory name. Refuses to overwrite a path that already exists.
///
/// # Errors
///
/// Returns an error when the target is missing or ambiguous, a destination
/// exists, or a file cannot be moved back.
pub fn restore_session(home: &Path, target: &str) -> Result<String> {
    let entry = find_entry(home, target)?;
    let manifest = read_manifest(&entry.join("session.json"))?;
    validate_restore(home, &entry, &manifest)?;
    for moved in &manifest.moved {
        if fs::symlink_metadata(&moved.from).is_ok() {
            bail!(
                "refusing to restore {} over existing {}",
                manifest.target,
                moved.from.display()
            );
        }
    }
    for moved in &manifest.moved {
        if let Some(parent) = moved.from.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to recreate {}", parent.display()))?;
        }
        let stored = entry.join(&moved.stored);
        fs::rename(&stored, &moved.from).with_context(|| {
            format!(
                "failed to restore {} to {}",
                stored.display(),
                moved.from.display()
            )
        })?;
    }
    restore_cursor_rows(&manifest.cursor_rows)?;
    if let Some(title) = manifest.overlay_title {
        titles::put(&titles::path_for(home), &manifest.target, Some(title))?;
    }
    fs::remove_dir_all(&entry)
        .with_context(|| format!("failed to remove trash entry {}", entry.display()))?;
    Ok(manifest.target)
}

fn validate_restore(home: &Path, entry: &Path, manifest: &Manifest) -> Result<()> {
    use super::adapter::ProviderAdapter;
    use crate::process::AgentKind;
    super::validate_storage_identifier(&manifest.id, "session ID")?;
    if let Some(title) = &manifest.overlay_title {
        titles::normalize(title)?;
    }
    anyhow::ensure!(
        manifest.target == format!("{}:{}", manifest.provider, manifest.id),
        "invalid trash identity"
    );
    let kind = AgentKind::from_slug(&manifest.provider).context("unknown trash provider")?;
    let roots = ProviderAdapter::from_kind(&kind)
        .context("unsupported trash provider")?
        .storage_roots(home);
    for moved in &manifest.moved {
        let name = moved
            .stored
            .strip_prefix("files/")
            .context("invalid stored artifact")?;
        anyhow::ensure!(
            !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit()),
            "invalid stored artifact"
        );
        let parent = entry.join("files").canonicalize()?;
        anyhow::ensure!(
            parent.starts_with(entry.canonicalize()?),
            "trash artifact escaped its entry"
        );
        validate_restore_destination(&roots, &moved.from)?;
    }
    for row in &manifest.cursor_rows {
        validate_restore_destination(&roots, &row.database)?;
    }
    Ok(())
}

fn validate_restore_destination(roots: &[PathBuf], path: &Path) -> Result<()> {
    // Missing session directories are expected; resolve the nearest existing
    // ancestor so a newly inserted symlink cannot redirect a restoration.
    fn resolve_missing(path: &Path) -> Result<PathBuf> {
        if fs::symlink_metadata(path).is_ok() {
            return Ok(path.canonicalize()?);
        }
        let parent = path.parent().context("invalid restore path")?;
        Ok(resolve_missing(parent)?.join(path.file_name().context("invalid restore name")?))
    }
    anyhow::ensure!(
        path.is_absolute()
            && path.components().count() <= 256
            && !path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir)),
        "unsafe restore destination"
    );
    let root = roots
        .iter()
        .find(|root| path.starts_with(root))
        .context("restore destination outside provider store")?;
    anyhow::ensure!(
        resolve_missing(path)?.starts_with(resolve_missing(root)?),
        "restore destination escaped provider store"
    );
    Ok(())
}

pub(super) fn state_dir(home: &Path) -> PathBuf {
    titles::path_for(home)
        .parent()
        .unwrap_or(home)
        .to_path_buf()
}

fn find_entry(home: &Path, target: &str) -> Result<PathBuf> {
    let root = state_dir(home).join("trash/sessions");
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            bail!("no trashed session matches {target}");
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", root.display()));
        }
    };
    let mut matches = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to read {}", root.display()))?;
        let path = entry.path();
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == target {
            matches.push(path);
            continue;
        }
        let Ok(manifest) = read_manifest(&path.join("session.json")) else {
            continue;
        };
        if manifest.target == target || manifest.id == target {
            matches.push(path);
        }
    }
    match matches.len() {
        0 => bail!("no trashed session matches {target}"),
        1 => Ok(matches.remove(0)),
        _ => bail!("trashed session `{target}` is ambiguous; pass the trash directory name"),
    }
}

fn read_manifest(path: &Path) -> Result<Manifest> {
    const LIMIT: u64 = 32 * 1024 * 1024;
    let metadata = fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= LIMIT,
        "invalid or oversized trash manifest"
    );
    let mut text = String::new();
    fs::File::open(path)?
        .take(LIMIT + 1)
        .read_to_string(&mut text)?;
    anyhow::ensure!(text.len() as u64 <= LIMIT, "oversized trash manifest");
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

#[derive(Debug, Clone)]
#[cfg(feature = "desktop")]
pub struct TrashedSession {
    pub key: String,
    pub target: String,
    pub provider: String,
    pub title: String,
    pub deleted_at: u64,
}

/// List mena-owned recovery manifests without reading provider stores.
///
/// # Errors
/// Returns an error when the trash directory cannot be read.
#[cfg(feature = "desktop")]
pub fn list_trashed_sessions(home: &Path) -> Result<Vec<TrashedSession>> {
    let root = state_dir(home).join("trash/sessions");
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut sessions = Vec::new();
    for entry in entries.take(20_000) {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Ok(manifest) = read_manifest(&entry.path().join("session.json")) else {
            continue;
        };
        sessions.push(TrashedSession {
            key: entry.file_name().to_string_lossy().into_owned(),
            title: manifest.title.unwrap_or_else(|| manifest.target.clone()),
            target: manifest.target,
            provider: manifest.provider,
            deleted_at: manifest.deleted_at,
        });
    }
    sessions.sort_by_key(|session| std::cmp::Reverse(session.deleted_at));
    Ok(sessions)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum SqliteValue {
    Null,
    Integer(i64),
    Real(u64),
    Text(String),
    Blob(String),
}

impl SqliteValue {
    pub(super) fn capture(value: rusqlite::types::ValueRef<'_>) -> Result<Self> {
        use rusqlite::types::ValueRef;
        Ok(match value {
            ValueRef::Null => Self::Null,
            ValueRef::Integer(value) => Self::Integer(value),
            ValueRef::Real(value) => Self::Real(value.to_bits()),
            ValueRef::Text(value) => Self::Text(std::str::from_utf8(value)?.to_owned()),
            ValueRef::Blob(value) => Self::Blob(encode_hex(value)),
        })
    }

    fn restore(&self) -> Result<rusqlite::types::Value> {
        use rusqlite::types::Value;
        Ok(match self {
            Self::Null => Value::Null,
            Self::Integer(value) => Value::Integer(*value),
            Self::Real(value) => Value::Real(f64::from_bits(*value)),
            Self::Text(value) => Value::Text(value.clone()),
            Self::Blob(value) => Value::Blob(decode_hex(value)?),
        })
    }
}

fn restore_cursor_rows(rows: &[CursorRow]) -> Result<()> {
    use rusqlite::{Connection, OpenFlags, types::Value};
    let mut databases = BTreeMap::<&Path, Vec<&CursorRow>>::new();
    for row in rows {
        databases.entry(&row.database).or_default().push(row);
    }
    for (database, rows) in databases {
        let mut connection = Connection::open_with_flags(
            database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("failed to open {}", database.display()))?;
        let transaction = connection.transaction()?;
        for row in rows {
            if !matches!(
                row.table.as_str(),
                "composerHeaders" | "cursorDiskKV" | "ItemTable"
            ) {
                bail!("refusing to restore unknown Cursor table {}", row.table);
            }
            let mut columns = row
                .columns
                .iter()
                .map(|(name, value)| Ok((name.clone(), value.restore()?)))
                .collect::<Result<BTreeMap<_, _>>>()?;
            if columns.is_empty() {
                // Backward compatibility with manifests written before typed row snapshots.
                let bytes = decode_hex(&row.value_hex)?;
                let (key_column, value) = if row.table == "composerHeaders" {
                    columns.insert(
                        "createdAt".to_owned(),
                        row.created_at.map_or(Value::Null, Value::Integer),
                    );
                    columns.insert(
                        "lastUpdatedAt".to_owned(),
                        row.updated_at.map_or(Value::Null, Value::Integer),
                    );
                    columns.insert(
                        "isArchived".to_owned(),
                        row.is_archived.map_or(Value::Null, Value::Integer),
                    );
                    ("composerId", Value::Text(String::from_utf8(bytes)?))
                } else {
                    ("key", Value::Blob(bytes))
                };
                columns.insert(key_column.to_owned(), Value::Text(row.key.clone()));
                columns.insert("value".to_owned(), value);
            }
            let names = columns
                .keys()
                .map(|name| format!("\"{}\"", name.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(", ");
            let placeholders = vec!["?"; columns.len()].join(", ");
            let sql = format!(
                "INSERT INTO {} ({names}) VALUES ({placeholders})",
                row.table
            );
            transaction
                .execute(&sql, rusqlite::params_from_iter(columns.values()))
                .with_context(|| {
                    format!(
                        "cannot restore {}:{}; an existing row is never overwritten",
                        row.table, row.key
                    )
                })?;
        }
        transaction.commit()?;
    }
    Ok(())
}

fn move_aside(from: &Path, to: &Path, symlink: bool) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::CrossesDevices => {
            if symlink {
                #[cfg(unix)]
                {
                    let target = fs::read_link(from)?;
                    std::os::unix::fs::symlink(target, to)?;
                    fs::remove_file(from)?;
                    return Ok(());
                }
                #[cfg(not(unix))]
                {
                    let _ = symlink;
                    return Err(error);
                }
            }
            fs::copy(from, to)?;
            fs::remove_file(from)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        bail!("odd hex length");
    }
    let mut bytes = Vec::with_capacity(text.len() / 2);
    let chars: Vec<u8> = text.bytes().collect();
    for pair in chars.chunks(2) {
        let high = hex_value(pair[0])?;
        let low = hex_value(pair[1])?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn hex_value(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => bail!("invalid hex digit"),
    }
}
