//! Move provider session files aside so `mena ss restore` can put them back.
//!
//! The trash directory lives next to mena’s other state (`trash/sessions`
//! under the config directory, or under a non-user home in tests). Native
//! index rows that can be reread are copied into the manifest before they
//! are removed. A failed deletion still writes the manifest for whatever
//! already moved.

use std::fs;
use std::io::{self, ErrorKind};
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
}

impl TrashBatch {
    pub(super) fn create(home: &Path, provider: &str, id: &str) -> Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis());
        let dir = state_dir(home)
            .join("trash/sessions")
            .join(format!("{stamp}-{provider}-{id}"));
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

    fn finish(mut self) -> Result<()> {
        if self.moved.is_empty() && self.cursor_rows.is_empty() {
            let _ = fs::remove_dir_all(&self.dir);
            self.written = true;
            return Ok(());
        }
        self.write_manifest()
    }

    fn write_manifest(&mut self) -> Result<()> {
        if self.written {
            return Ok(());
        }
        let manifest = Manifest {
            target: format!("{}:{}", self.provider, self.id),
            provider: self.provider.clone(),
            id: self.id.clone(),
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

pub(super) fn finish(batch: TrashBatch) -> Result<()> {
    batch.finish()
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
    for moved in &manifest.moved {
        if moved.from.exists() {
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
    fs::remove_dir_all(&entry)
        .with_context(|| format!("failed to remove trash entry {}", entry.display()))?;
    Ok(manifest.target)
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
        if !path.is_dir() {
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
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

fn restore_cursor_rows(rows: &[CursorRow]) -> Result<()> {
    use rusqlite::{Connection, OpenFlags, params};
    for row in rows {
        if !row.database.is_file() {
            bail!(
                "cannot restore Cursor row; database is gone: {}",
                row.database.display()
            );
        }
        let connection = Connection::open_with_flags(
            &row.database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("failed to open {}", row.database.display()))?;
        let bytes = decode_hex(&row.value_hex)
            .with_context(|| format!("trashed Cursor value for {} is not hex", row.key))?;
        match row.table.as_str() {
            "composerHeaders" => {
                connection
                    .execute(
                        "INSERT OR IGNORE INTO composerHeaders (composerId, createdAt, lastUpdatedAt, isArchived, value) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![row.key, row.created_at, row.updated_at, row.is_archived, bytes],
                    )
                    .with_context(|| format!("failed to restore composerHeaders in {}", row.database.display()))?;
            }
            "cursorDiskKV" | "ItemTable" => {
                let sql = format!(
                    "INSERT OR IGNORE INTO {} (key, value) VALUES (?1, ?2)",
                    row.table
                );
                connection
                    .execute(&sql, params![row.key, bytes])
                    .with_context(|| {
                        format!(
                            "failed to restore {} in {}",
                            row.table,
                            row.database.display()
                        )
                    })?;
            }
            other => bail!("refusing to restore unknown Cursor table {other}"),
        }
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
