//! `History.db`: every finished upload, kept on this computer (ShareX.HistoryLib
//! `HistoryManager`, `HistoryManagerSQLite`, `HistoryItem`, and the reading half of
//! `HistoryManagerJSON` for the old `History.json`).
//!
//! The file is the Windows app's file: the same table, columns and statements, and the
//! same text in every cell, so a History.db written by either app reads in the other.
//! Three .NET habits shape the cells and are kept on purpose:
//!
//! * `DateTime` holds `DateTime.ToString("o")` of a local time,
//!   `2026-10-19T22:30:12.1234567+02:00`, and is read the way `DateTime.Parse` reads it
//!   (an offset, a trailing `Z`, or no zone at all, which is a local time).
//! * A NULL cell reads back as an empty string, as `DBNull.ToString()` does.
//! * `Tags` is Newtonsoft's JSON of a `Dictionary<string, string>`, NULL when the item
//!   has no dictionary at all.
//!
//! Errors are returned, never shown: upstream's `GetHistoryItems` puts the exception in a
//! message box, and that is the caller's job here.

use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use chrono::{
    DateTime, Datelike, Local, LocalResult, NaiveDate, NaiveDateTime, TimeDelta, TimeZone,
    Timelike, Utc,
};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, Row, TransactionBehavior, named_params};
use serde_json::{Map, Value};

/// `Program.HistoryFileName`, in the personal folder.
pub const HISTORY_FILE_NAME: &str = "History.db";

/// `Program.HistoryFileNameOld`: the JSON history that came before the database.
pub const HISTORY_FILE_NAME_OLD: &str = "History.json";

/// `HistoryManagerSQLite.SetBusyTimeout(5000)`: how long a statement waits for another
/// connection (the History window, a second app instance) to let go of the file.
const BUSY_TIMEOUT_MS: u32 = 5000;

/// `EnsureDatabase`, character for character. The line breaks are CRLF because upstream's
/// `.cs` files are checked out with CRLF and the statement is a verbatim string: SQLite
/// keeps this text in `sqlite_master`, so a file created here matches one created there.
const CREATE_TABLE: &str = "\r\nCREATE TABLE IF NOT EXISTS History (\r\n    Id INTEGER PRIMARY KEY AUTOINCREMENT,\r\n    FileName TEXT,\r\n    FilePath TEXT,\r\n    DateTime TEXT,\r\n    Type TEXT,\r\n    Host TEXT,\r\n    URL TEXT,\r\n    ThumbnailURL TEXT,\r\n    DeletionURL TEXT,\r\n    ShortenedURL TEXT,\r\n    Tags TEXT\r\n);\r\n";

const INSERT: &str = "
INSERT INTO History
(FileName, FilePath, DateTime, Type, Host, URL, ThumbnailURL, DeletionURL, ShortenedURL, Tags)
VALUES (@FileName, @FilePath, @DateTime, @Type, @Host, @URL, @ThumbnailURL, @DeletionURL, @ShortenedURL, @Tags);";

const UPDATE: &str = "
UPDATE History SET
FileName = @FileName,
FilePath = @FilePath,
DateTime = @DateTime,
Type = @Type,
Host = @Host,
URL = @URL,
ThumbnailURL = @ThumbnailURL,
DeletionURL = @DeletionURL,
ShortenedURL = @ShortenedURL,
Tags = @Tags
WHERE Id = @Id;";

const DELETE: &str = "DELETE FROM History WHERE Id = @Id;";

/// `DateTime.MinValue.ToString("o")`: what upstream writes for an item that has no time
/// (only the JSON migration ever does, as it skips the validity check).
const MIN_VALUE_TEXT: &str = "0001-01-01T00:00:00.0000000";

/// The `Tags` keys upstream has accessors for.
const TAG_WINDOW_TITLE: &str = "WindowTitle";
const TAG_PROCESS_NAME: &str = "ProcessName";
const TAG_FAVORITE: &str = "Favorite";
const TAG_TAG: &str = "Tag";

/// Why the history could not be read or written.
#[derive(Debug)]
pub enum HistoryError {
    /// The folder of the file could not be created, or `History.json` could not be read.
    Io(std::io::Error),
    /// SQLite failed: not a database, held by another connection for longer than the busy
    /// timeout, a full disk...
    Sqlite(rusqlite::Error),
    /// `History.json` is not the JSON upstream writes.
    Json(serde_json::Error),
    /// A stored value the reader cannot take. Upstream's `Load` throws on the same values,
    /// which makes the whole history unreadable there too.
    Invalid {
        /// Where: `row 12, DateTime` or `History.json entry 3, Tags`.
        location: String,
        detail: String,
    },
}

impl HistoryError {
    fn invalid(location: impl Into<String>, detail: impl Into<String>) -> HistoryError {
        HistoryError::Invalid {
            location: location.into(),
            detail: detail.into(),
        }
    }
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HistoryError::Io(error) => write!(f, "{error}"),
            HistoryError::Sqlite(error) => write!(f, "{error}"),
            HistoryError::Json(error) => write!(f, "{error}"),
            HistoryError::Invalid { location, detail } => write!(f, "{location}: {detail}"),
        }
    }
}

impl std::error::Error for HistoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HistoryError::Io(error) => Some(error),
            HistoryError::Sqlite(error) => Some(error),
            HistoryError::Json(error) => Some(error),
            HistoryError::Invalid { .. } => None,
        }
    }
}

impl From<std::io::Error> for HistoryError {
    fn from(error: std::io::Error) -> Self {
        HistoryError::Io(error)
    }
}

impl From<rusqlite::Error> for HistoryError {
    fn from(error: rusqlite::Error) -> Self {
        HistoryError::Sqlite(error)
    }
}

/// `HistoryItem.Tags`: a `Dictionary<string, string>` whose values may be null, kept in
/// the order the entries were added, which is the order Newtonsoft writes them in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryTags {
    entries: Vec<(String, Option<String>)>,
}

impl HistoryTags {
    pub fn new() -> HistoryTags {
        HistoryTags::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Keys compare like the dictionary's default comparer: ordinal, case-sensitive.
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    /// `None` when the key is absent, `Some(None)` when it is there with a null value.
    pub fn get(&self, key: &str) -> Option<Option<&str>> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_deref())
    }

    /// The indexer's set: an existing key keeps its place, a new one goes last.
    pub fn insert(&mut self, key: impl Into<String>, value: Option<String>) {
        let key = key.into();
        match self.entries.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = value,
            None => self.entries.push((key, value)),
        }
    }

    /// Whether the key was there.
    pub fn remove(&mut self, key: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(k, _)| k != key);
        self.entries.len() != before
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, Option<&str>)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_deref()))
    }

    /// `JsonConvert.SerializeObject(Tags)`, byte for byte: compact, and escaped the way
    /// Newtonsoft escapes by default.
    fn to_json(&self) -> String {
        let mut out = String::from("{");
        for (index, (key, value)) in self.entries.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            write_json_string(&mut out, key);
            out.push(':');
            match value {
                Some(value) => write_json_string(&mut out, value),
                None => out.push_str("null"),
            }
        }
        out.push('}');
        out
    }

    /// `JsonConvert.DeserializeObject<Dictionary<string, string>>(cell)`. An empty cell
    /// (what a NULL turns into) and `null` give no dictionary at all.
    fn from_json(text: &str) -> Result<Option<HistoryTags>, String> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
        tags_from_value(&value)
    }
}

fn tags_from_value(value: &Value) -> Result<Option<HistoryTags>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Object(map) => {
            let mut tags = HistoryTags::new();
            for (key, value) in map {
                tags.insert(key.clone(), json_text(value)?);
            }
            Ok(Some(tags))
        }
        other => Err(format!("expected an object of strings, found {other}")),
    }
}

/// A JSON scalar read into a string as Newtonsoft's `ReadAsString` does: numbers and
/// booleans become their text; objects and arrays are refused.
fn json_text(value: &Value) -> Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        Value::Bool(flag) => Ok(Some(flag.to_string())),
        Value::Number(number) => Ok(Some(number.to_string())),
        other => Err(format!("expected a string, found {other}")),
    }
}

/// Newtonsoft's default string escaping: the two-character escapes, `\u` with lowercase
/// hex for the other control characters and for U+0085, U+2028 and U+2029, and everything
/// else (all other non-ASCII text included) as it is.
fn write_json_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || matches!(c, '\u{85}' | '\u{2028}' | '\u{2029}') => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// One row of `History` (`HistoryItem`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryItem {
    /// The row id; 0 until the item is stored.
    pub id: i64,
    pub file_name: String,
    pub file_path: String,
    /// When the task finished. `None` is .NET's `DateTime.MinValue`, "not set".
    pub date_time: Option<DateTime<Utc>>,
    /// `Type`: "Image", "Text", "File", ...
    pub kind: String,
    pub host: String,
    pub url: String,
    pub thumbnail_url: String,
    pub deletion_url: String,
    pub shortened_url: String,
    /// `None` is upstream's null dictionary (a NULL cell), which is not the same cell as
    /// an empty `{}`.
    pub tags: Option<HistoryTags>,
}

impl HistoryItem {
    /// `HistoryManager.IsValidHistoryItem`: a name, a time, and somewhere to find the
    /// file. A time .NET's `DateTime` cannot hold (outside the years 1 to 9999 on this
    /// computer's clock) counts as no time, since the Windows app could not read it back.
    pub fn is_valid(&self) -> bool {
        !self.file_name.is_empty()
            && self.date_time.is_some_and(|t| dotnet_text(t).is_some())
            && (!self.url.is_empty() || !self.file_path.is_empty())
    }

    /// `TagsWindowTitle`.
    pub fn tags_window_title(&self) -> Option<&str> {
        self.tag_value(TAG_WINDOW_TITLE)
    }

    /// `TagsProcessName`.
    pub fn tags_process_name(&self) -> Option<&str> {
        self.tag_value(TAG_PROCESS_NAME)
    }

    /// `Favorite`: the key is there, whatever its value.
    pub fn favorite(&self) -> bool {
        self.tags
            .as_ref()
            .is_some_and(|tags| tags.contains_key(TAG_FAVORITE))
    }

    /// Upstream's setter: a favourite is the key with a null value. Like upstream, either
    /// way leaves the item with a dictionary, so it is written as `{}` at the least.
    pub fn set_favorite(&mut self, value: bool) {
        let tags = self.tags.get_or_insert_with(HistoryTags::new);
        if value {
            tags.insert(TAG_FAVORITE, None);
        } else {
            tags.remove(TAG_FAVORITE);
        }
    }

    /// `Tag`: the user's own label.
    pub fn tag(&self) -> Option<&str> {
        self.tag_value(TAG_TAG)
    }

    /// Upstream's setter: an empty label removes the key.
    pub fn set_tag(&mut self, value: &str) {
        let tags = self.tags.get_or_insert_with(HistoryTags::new);
        if value.is_empty() {
            tags.remove(TAG_TAG);
        } else {
            tags.insert(TAG_TAG, Some(value.to_string()));
        }
    }

    fn tag_value(&self, key: &str) -> Option<&str> {
        self.tags.as_ref().and_then(|tags| tags.get(key)).flatten()
    }
}

/// How an empty string goes into a cell. Rust has no null string, so the two upstream
/// paths are told apart: a new item's missing values are null upstream and go in as
/// NULL; a loaded item has `""` where the cell was NULL (`DBNull.ToString()`), and
/// upstream's `Edit` writes those back as `''`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EmptyText {
    Null,
    Empty,
}

fn cell(value: &str, empty: EmptyText) -> Option<&str> {
    if value.is_empty() && empty == EmptyText::Null {
        None
    } else {
        Some(value)
    }
}

/// The history store (`HistoryManagerSQLite`): one connection to `History.db`, shared by
/// every thread that holds the store.
#[derive(Debug)]
pub struct HistoryStore {
    path: PathBuf,
    connection: Mutex<Connection>,
}

impl HistoryStore {
    /// `new HistoryManagerSQLite(filePath)`: create the folder, open (or create) the
    /// file, set the busy timeout and make sure the table exists. A file that is not a
    /// database, or one locked for longer than the busy timeout, is an error.
    pub fn open(path: impl AsRef<Path>) -> Result<HistoryStore, HistoryError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        // No SQLITE_OPEN_URI: the path is a file name, never a `file:` URI.
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = Connection::open_with_flags(&path, flags)?;
        connection.execute_batch(&format!("PRAGMA busy_timeout = {BUSY_TIMEOUT_MS};"))?;
        connection.execute_batch(CREATE_TABLE)?;
        Ok(HistoryStore {
            path,
            connection: Mutex::new(connection),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The connection. A thread that panicked while holding it cannot have left a
    /// transaction open (rusqlite rolls back on drop), so a poisoned lock is still good.
    fn connection(&self) -> MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// `Load`: every row, in the table's order. One unreadable value fails the whole
    /// read, as it does upstream.
    pub fn load(&self) -> Result<Vec<HistoryItem>, HistoryError> {
        let connection = self.connection();
        let mut statement = connection.prepare("SELECT * FROM History;")?;
        let columns = Columns::find(&statement)?;
        let mut rows = statement.query([])?;
        let mut items = Vec::new();
        while let Some(row) = rows.next()? {
            items.push(columns.read(row)?);
        }
        Ok(items)
    }

    /// `AppendHistoryItems`: the valid items (see [`HistoryItem::is_valid`]) in one
    /// transaction; the others are skipped without a word, as upstream skips them. The
    /// stored items get their row ids. Returns how many were stored.
    pub fn append(&self, items: &mut [HistoryItem]) -> Result<usize, HistoryError> {
        let mut valid: Vec<&mut HistoryItem> =
            items.iter_mut().filter(|item| item.is_valid()).collect();
        self.insert(&mut valid)?;
        Ok(valid.len())
    }

    /// `AppendHistoryItem`: whether the item was valid and is now stored.
    pub fn append_item(&self, item: &mut HistoryItem) -> Result<bool, HistoryError> {
        Ok(self.append(std::slice::from_mut(item))? == 1)
    }

    /// The inserts of upstream's `Append`, in one immediate transaction (what
    /// Microsoft.Data.Sqlite's `BeginTransaction()` starts). The ids are handed out only
    /// once the transaction has committed, so a failed append leaves the items as they
    /// were.
    fn insert(&self, items: &mut [&mut HistoryItem]) -> Result<(), HistoryError> {
        if items.is_empty() {
            return Ok(());
        }
        let mut times = Vec::with_capacity(items.len());
        for item in items.iter() {
            times.push(date_time_cell(item)?);
        }
        let mut connection = self.connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut ids = Vec::with_capacity(items.len());
        {
            let mut statement = transaction.prepare(INSERT)?;
            for (item, date_time) in items.iter().zip(&times) {
                let tags = item.tags.as_ref().map(HistoryTags::to_json);
                ids.push(statement.insert(named_params! {
                    "@FileName": cell(&item.file_name, EmptyText::Null),
                    "@FilePath": cell(&item.file_path, EmptyText::Null),
                    "@DateTime": date_time,
                    "@Type": cell(&item.kind, EmptyText::Null),
                    "@Host": cell(&item.host, EmptyText::Null),
                    "@URL": cell(&item.url, EmptyText::Null),
                    "@ThumbnailURL": cell(&item.thumbnail_url, EmptyText::Null),
                    "@DeletionURL": cell(&item.deletion_url, EmptyText::Null),
                    "@ShortenedURL": cell(&item.shortened_url, EmptyText::Null),
                    "@Tags": tags,
                })?);
            }
        }
        transaction.commit()?;
        for (item, id) in items.iter_mut().zip(ids) {
            item.id = id;
        }
        Ok(())
    }

    /// `Edit`: write every column of the row with the item's id. Like upstream, there is
    /// no validity check, and an id with no row changes nothing.
    pub fn edit(&self, item: &HistoryItem) -> Result<(), HistoryError> {
        let date_time = date_time_cell(item)?;
        let tags = item.tags.as_ref().map(HistoryTags::to_json);
        let mut connection = self.connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            UPDATE,
            named_params! {
                "@FileName": cell(&item.file_name, EmptyText::Empty),
                "@FilePath": cell(&item.file_path, EmptyText::Empty),
                "@DateTime": date_time,
                "@Type": cell(&item.kind, EmptyText::Empty),
                "@Host": cell(&item.host, EmptyText::Empty),
                "@URL": cell(&item.url, EmptyText::Empty),
                "@ThumbnailURL": cell(&item.thumbnail_url, EmptyText::Empty),
                "@DeletionURL": cell(&item.deletion_url, EmptyText::Empty),
                "@ShortenedURL": cell(&item.shortened_url, EmptyText::Empty),
                "@Tags": tags,
                "@Id": item.id,
            },
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// `Delete`: the rows of these items, in one transaction.
    pub fn delete(&self, items: &[HistoryItem]) -> Result<(), HistoryError> {
        if items.is_empty() {
            return Ok(());
        }
        let mut connection = self.connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut statement = transaction.prepare(DELETE)?;
            for item in items {
                statement.execute(named_params! { "@Id": item.id })?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// `MigrateFromJSON`: copy the old `History.json` into this store. Upstream hands the
    /// items to `Append` directly, past `IsValidHistoryItem`, so nothing is filtered here
    /// either. Returns how many items were copied. Moving the JSON file to the backup
    /// folder afterwards is the caller's part (`SettingManager.MigrateHistoryFile`).
    pub fn migrate_from_json(&self, json_path: impl AsRef<Path>) -> Result<usize, HistoryError> {
        let mut items = read_json_history(json_path.as_ref())?;
        let mut all: Vec<&mut HistoryItem> = items.iter_mut().collect();
        self.insert(&mut all)?;
        Ok(all.len())
    }
}

/// The `DateTime` cell of an item: `ToString("o")` of the local time, or MinValue's text.
fn date_time_cell(item: &HistoryItem) -> Result<String, HistoryError> {
    match item.date_time {
        None => Ok(MIN_VALUE_TEXT.to_string()),
        Some(instant) => dotnet_text(instant).ok_or_else(|| {
            HistoryError::invalid(
                format!("row {}, DateTime", item.id),
                format!("{instant} is outside the years .NET can store"),
            )
        }),
    }
}

/// The positions of upstream's columns in `SELECT *`, looked up by name as
/// `reader["FileName"]` does, so a table with more columns still reads.
struct Columns {
    id: usize,
    file_name: usize,
    file_path: usize,
    date_time: usize,
    kind: usize,
    host: usize,
    url: usize,
    thumbnail_url: usize,
    deletion_url: usize,
    shortened_url: usize,
    tags: usize,
}

impl Columns {
    fn find(statement: &rusqlite::Statement<'_>) -> rusqlite::Result<Columns> {
        Ok(Columns {
            id: statement.column_index("Id")?,
            file_name: statement.column_index("FileName")?,
            file_path: statement.column_index("FilePath")?,
            date_time: statement.column_index("DateTime")?,
            kind: statement.column_index("Type")?,
            host: statement.column_index("Host")?,
            url: statement.column_index("URL")?,
            thumbnail_url: statement.column_index("ThumbnailURL")?,
            deletion_url: statement.column_index("DeletionURL")?,
            shortened_url: statement.column_index("ShortenedURL")?,
            tags: statement.column_index("Tags")?,
        })
    }

    fn read(&self, row: &Row<'_>) -> Result<HistoryItem, HistoryError> {
        let id: i64 = row.get(self.id)?;
        let date_time = parse_date_time(&text(row, self.date_time)?)
            .map_err(|detail| HistoryError::invalid(format!("row {id}, DateTime"), detail))?;
        let tags = HistoryTags::from_json(&text(row, self.tags)?)
            .map_err(|detail| HistoryError::invalid(format!("row {id}, Tags"), detail))?;
        Ok(HistoryItem {
            id,
            file_name: text(row, self.file_name)?,
            file_path: text(row, self.file_path)?,
            date_time,
            kind: text(row, self.kind)?,
            host: text(row, self.host)?,
            url: text(row, self.url)?,
            thumbnail_url: text(row, self.thumbnail_url)?,
            deletion_url: text(row, self.deletion_url)?,
            shortened_url: text(row, self.shortened_url)?,
            tags,
        })
    }
}

/// `reader[column].ToString()`: NULL is `""` (`DBNull.ToString()`), a number its text.
/// Bytes that are not UTF-8 become U+FFFD, as .NET's UTF-8 decoder makes them.
fn text(row: &Row<'_>, index: usize) -> rusqlite::Result<String> {
    Ok(match row.get_ref(index)? {
        ValueRef::Null => String::new(),
        ValueRef::Integer(value) => value.to_string(),
        ValueRef::Real(value) => value.to_string(),
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
            String::from_utf8_lossy(bytes).into_owned()
        }
    })
}

/// `DateTime.ToString("o")` of the instant as a local time: the wall clock of this
/// computer's zone, seven fraction digits, and the offset as `zzz` writes it (hours and
/// minutes only). The wall time is built from that same whole-minute offset, so the text
/// names exactly the instant it was made from. `None` outside the years .NET can hold.
fn dotnet_text(instant: DateTime<Utc>) -> Option<String> {
    let utc = instant.naive_utc();
    let offset_minutes = Local.offset_from_utc_datetime(&utc).local_minus_utc() / 60;
    let wall = utc.checked_add_signed(TimeDelta::minutes(i64::from(offset_minutes)))?;
    if !(1..=9999).contains(&wall.year()) {
        return None;
    }
    // Leap seconds live in chrono's nanoseconds past 10^9; .NET has none.
    let ticks = wall.nanosecond() % 1_000_000_000 / 100;
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let offset = offset_minutes.unsigned_abs();
    Some(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:07}{}{:02}:{:02}",
        wall.year(),
        wall.month(),
        wall.day(),
        wall.hour(),
        wall.minute(),
        wall.second(),
        ticks,
        sign,
        offset / 60,
        offset % 60,
    ))
}

/// What follows the time in a `DateTime` cell.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Zone {
    /// Nothing: `DateTime.Parse` gives a local time (`DateTimeKind.Unspecified`).
    Local,
    /// `Z`.
    Utc,
    /// `+02:00`, in seconds east of UTC.
    Offset(i32),
}

/// A `DateTime` cell read the way `DateTime.Parse` reads what `ToString("o")` and
/// Newtonsoft (in History.json) write: `yyyy-MM-ddTHH:mm:ss.fffffff` and then an offset,
/// `Z`, or nothing for a local time. Any number of fraction digits, a missing fraction,
/// missing seconds or a missing time are taken too, and so is a space for the `T`.
/// `Ok(None)` is `DateTime.MinValue` without a zone, upstream's "no time".
fn parse_date_time(text: &str) -> Result<Option<DateTime<Utc>>, String> {
    let error = || format!("String '{text}' was not recognized as a valid DateTime.");
    let mut cursor = Cursor::new(text.trim());
    let year = cursor.number(4).ok_or_else(error)?;
    let month = cursor.field(b'-').ok_or_else(error)?;
    let day = cursor.field(b'-').ok_or_else(error)?;
    let (mut hour, mut minute, mut second, mut ticks) = (0, 0, 0, 0);
    let mut zone = Zone::Local;
    if cursor.take_any(b"Tt ").is_some() {
        hour = cursor.number(2).ok_or_else(error)?;
        minute = cursor.field(b':').ok_or_else(error)?;
        if let Some(value) = cursor.field(b':') {
            second = value;
            if cursor.eat(b'.') {
                ticks = cursor.fraction_ticks().ok_or_else(error)?;
            }
        }
        if cursor.take_any(b"Zz").is_some() {
            zone = Zone::Utc;
        } else if let Some(sign) = cursor.take_any(b"+-") {
            let hours = cursor.number(2).ok_or_else(error)?;
            cursor.eat(b':');
            let minutes = cursor.number(2).unwrap_or(0);
            if hours > 14 || minutes > 59 {
                return Err(error());
            }
            let seconds = (hours * 3600 + minutes * 60) as i32;
            zone = Zone::Offset(if sign == b'-' { -seconds } else { seconds });
        }
    }
    if !cursor.is_empty() {
        return Err(error());
    }
    let is_min_value = (year, month, day, hour, minute, second, ticks) == (1, 1, 1, 0, 0, 0, 0);
    if is_min_value && zone == Zone::Local {
        return Ok(None);
    }
    let naive = NaiveDate::from_ymd_opt(year as i32, month, day)
        .and_then(|date| date.and_hms_opt(hour, minute, second))
        .and_then(|time| time.checked_add_signed(TimeDelta::nanoseconds(i64::from(ticks) * 100)))
        .ok_or_else(error)?;
    Ok(Some(match zone {
        Zone::Local => local_to_utc(naive),
        Zone::Utc => naive.and_utc(),
        Zone::Offset(seconds) => naive
            .checked_sub_signed(TimeDelta::seconds(i64::from(seconds)))
            .ok_or_else(error)?
            .and_utc(),
    }))
}

/// A local wall time as an instant, resolved the way .NET's `TimeZoneInfo` resolves the
/// hours a clock change makes odd: a repeated hour is standard time (the later instant),
/// a skipped hour is daylight time (the offset in force after the jump).
fn local_to_utc(naive: NaiveDateTime) -> DateTime<Utc> {
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(time) => time.with_timezone(&Utc),
        LocalResult::Ambiguous(_, later) => later.with_timezone(&Utc),
        LocalResult::None => {
            let after = naive
                .checked_add_signed(TimeDelta::days(1))
                .map(|probe| Local.offset_from_utc_datetime(&probe).local_minus_utc())
                .unwrap_or(0);
            naive
                .checked_sub_signed(TimeDelta::seconds(i64::from(after)))
                .unwrap_or(naive)
                .and_utc()
        }
    }
}

/// A forward-only reader over the ASCII of a `DateTime` cell.
struct Cursor<'a> {
    rest: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str) -> Cursor<'a> {
        Cursor {
            rest: text.as_bytes(),
        }
    }

    fn is_empty(&self) -> bool {
        self.rest.is_empty()
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.take_any(&[byte]).is_some()
    }

    fn take_any(&mut self, bytes: &[u8]) -> Option<u8> {
        let (&first, rest) = self.rest.split_first()?;
        if bytes.contains(&first) {
            self.rest = rest;
            Some(first)
        } else {
            None
        }
    }

    /// Exactly `digits` decimal digits.
    fn number(&mut self, digits: usize) -> Option<u32> {
        let field = self.rest.get(..digits)?;
        if !field.iter().all(u8::is_ascii_digit) {
            return None;
        }
        self.rest = &self.rest[digits..];
        Some(field.iter().fold(0, |n, d| n * 10 + u32::from(d - b'0')))
    }

    /// A separator and then two digits.
    fn field(&mut self, separator: u8) -> Option<u32> {
        if self.eat(separator) {
            self.number(2)
        } else {
            None
        }
    }

    /// A fraction of a second in .NET ticks (100 ns): seven digits are exact, fewer are
    /// padded, more are rounded to the nearest tick.
    fn fraction_ticks(&mut self) -> Option<u32> {
        let count = self.rest.iter().take_while(|b| b.is_ascii_digit()).count();
        if count == 0 {
            return None;
        }
        let (digits, rest) = self.rest.split_at(count);
        self.rest = rest;
        let mut ticks = 0;
        for place in 0..7 {
            let digit = digits.get(place).map_or(0, |d| u32::from(d - b'0'));
            ticks = ticks * 10 + digit;
        }
        if digits.get(7).is_some_and(|d| *d >= b'5') {
            ticks += 1;
        }
        Some(ticks)
    }
}

/// `HistoryManagerJSON.Load`: the old `History.json`, a run of JSON objects separated by
/// commas (the file is wrapped in `[` `]` to read it). A missing or empty file is an
/// empty history.
pub fn read_json_history(path: &Path) -> Result<Vec<HistoryItem>, HistoryError> {
    if path.as_os_str().is_empty() || !path.is_file() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(path)?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let text = String::from_utf8_lossy(bytes);
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let entries: Vec<Value> =
        serde_json::from_str(&format!("[{text}]")).map_err(HistoryError::Json)?;
    let mut items = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        match entry {
            Value::Object(map) => items.push(item_from_json(index, map)?),
            // Upstream's writer never writes one; there is nothing to keep from it.
            Value::Null => {}
            other => {
                return Err(HistoryError::invalid(
                    format!("{HISTORY_FILE_NAME_OLD} entry {index}"),
                    format!("expected an object, found {other}"),
                ));
            }
        }
    }
    Ok(items)
}

/// One History.json object as Newtonsoft reads it into a `HistoryItem`: members matched
/// by name, exactly first and then ignoring case; a missing member keeps its default.
fn item_from_json(index: usize, map: &Map<String, Value>) -> Result<HistoryItem, HistoryError> {
    let member = |name: &str| {
        map.get(name).or_else(|| {
            map.iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value)
        })
    };
    let invalid = |name: &str, detail: String| {
        HistoryError::invalid(
            format!("{HISTORY_FILE_NAME_OLD} entry {index}, {name}"),
            detail,
        )
    };
    let text = |name: &str| -> Result<String, HistoryError> {
        match member(name) {
            Some(value) => json_text(value)
                .map(Option::unwrap_or_default)
                .map_err(|detail| invalid(name, detail)),
            None => Ok(String::new()),
        }
    };
    let date_time = match member("DateTime") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => {
            parse_date_time(value).map_err(|detail| invalid("DateTime", detail))?
        }
        Some(other) => {
            return Err(invalid(
                "DateTime",
                format!("expected a date, found {other}"),
            ));
        }
    };
    let tags = match member("Tags") {
        None => None,
        Some(value) => tags_from_value(value).map_err(|detail| invalid("Tags", detail))?,
    };
    Ok(HistoryItem {
        id: 0,
        file_name: text("FileName")?,
        file_path: text("FilePath")?,
        date_time,
        kind: text("Type")?,
        host: text("Host")?,
        url: text("URL")?,
        thumbnail_url: text("ThumbnailURL")?,
        deletion_url: text("DeletionURL")?,
        shortened_url: text("ShortenedURL")?,
        tags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        let naive = NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap();
        Local
            .from_local_datetime(&naive)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }

    fn uploaded(name: &str) -> HistoryItem {
        HistoryItem {
            file_name: name.to_string(),
            file_path: format!("/Users/someone/Pictures/{name}"),
            date_time: Some(at("2026-10-19T20:30:12.1234567Z")),
            kind: "Image".to_string(),
            host: "Modlight".to_string(),
            url: format!("https://example.com/{name}"),
            thumbnail_url: format!("https://example.com/t/{name}"),
            ..HistoryItem::default()
        }
    }

    fn raw(path: &Path) -> Connection {
        Connection::open(path).unwrap()
    }

    /// The ten data cells of a row as SQLite holds them (FileName first), NULL as `None`.
    fn cells(connection: &Connection, id: i64) -> Vec<Option<String>> {
        connection
            .query_row("SELECT * FROM History WHERE Id = ?1", [id], |row| {
                (1..11).map(|i| row.get::<_, Option<String>>(i)).collect()
            })
            .unwrap()
    }

    #[test]
    fn items_round_trip_and_get_their_ids() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::open(dir.path().join("sub").join(HISTORY_FILE_NAME)).unwrap();
        let mut first = uploaded("a.png");
        first.deletion_url = "https://example.com/delete/a".to_string();
        first.shortened_url = "https://exa.mp/a".to_string();
        first.set_favorite(true);
        let mut items = vec![first, uploaded("ż ółw.png")];
        assert_eq!(store.append(&mut items).unwrap(), 2);
        assert_eq!((items[0].id, items[1].id), (1, 2));
        assert_eq!(store.load().unwrap(), items);
    }

    #[test]
    fn invalid_items_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::open(dir.path().join(HISTORY_FILE_NAME)).unwrap();
        let no_name = HistoryItem {
            file_name: String::new(),
            ..uploaded("x")
        };
        let no_time = HistoryItem {
            date_time: None,
            ..uploaded("x")
        };
        let nowhere = HistoryItem {
            url: String::new(),
            file_path: String::new(),
            ..uploaded("x")
        };
        let beyond_dotnet = HistoryItem {
            date_time: Some(Utc.with_ymd_and_hms(10001, 1, 1, 0, 0, 0).unwrap()),
            ..uploaded("x")
        };
        let path_only = HistoryItem {
            url: String::new(),
            ..uploaded("path-only.png")
        };
        let url_only = HistoryItem {
            file_path: String::new(),
            ..uploaded("url-only.png")
        };
        let mut items = vec![
            no_name,
            no_time,
            nowhere,
            beyond_dotnet,
            path_only,
            url_only,
        ];
        assert_eq!(store.append(&mut items).unwrap(), 2);
        let ids: Vec<i64> = items.iter().map(|i| i.id).collect();
        assert_eq!(ids, [0, 0, 0, 0, 1, 2]);
        let names: Vec<String> = store
            .load()
            .unwrap()
            .into_iter()
            .map(|i| i.file_name)
            .collect();
        assert_eq!(names, ["path-only.png", "url-only.png"]);

        let mut lone = HistoryItem::default();
        assert!(!store.append_item(&mut lone).unwrap());
        assert_eq!(lone.id, 0);
    }

    #[test]
    fn null_cells_read_back_as_empty_strings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE_NAME);
        let store = HistoryStore::open(&path).unwrap();
        // What upstream appends for a queued upload: FilePath, Type, DeletionURL,
        // ShortenedURL and Tags were null.
        let mut item = HistoryItem {
            file_path: String::new(),
            kind: String::new(),
            ..uploaded("q.png")
        };
        store.append_item(&mut item).unwrap();
        let stored = cells(&raw(&path), item.id);
        assert_eq!(stored[0].as_deref(), Some("q.png"));
        assert_eq!(stored[1], None, "FilePath");
        assert_eq!(stored[3], None, "Type");
        assert_eq!(stored[7], None, "DeletionURL");
        assert_eq!(stored[8], None, "ShortenedURL");
        assert_eq!(stored[9], None, "Tags");

        // A row with nothing but a name and a time.
        raw(&path)
            .execute(
                "INSERT INTO History (FileName, DateTime) VALUES ('bare.png', '2026-01-15T10:00:00.0000000Z')",
                [],
            )
            .unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded[0], item);
        let bare = &loaded[1];
        assert_eq!(bare.file_name, "bare.png");
        assert_eq!((bare.file_path.as_str(), bare.url.as_str()), ("", ""));
        assert_eq!(bare.tags, None);

        // Upstream's Edit writes a loaded item's empty strings back as '' (they are not
        // null any more), and no reader can tell the two apart.
        store.edit(bare).unwrap();
        let stored = cells(&raw(&path), bare.id);
        assert_eq!(stored[1].as_deref(), Some(""));
        assert_eq!(stored[9], None, "no dictionary stays NULL");
        assert_eq!(store.load().unwrap()[1], *bare);
    }

    #[test]
    fn edit_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::open(dir.path().join(HISTORY_FILE_NAME)).unwrap();
        let mut items = vec![uploaded("a.png"), uploaded("b.png"), uploaded("c.png")];
        store.append(&mut items).unwrap();

        let mut b = items[1].clone();
        b.set_tag("holiday");
        b.url = "https://example.com/other".to_string();
        b.date_time = Some(at("2025-01-15T08:00:00Z"));
        store.edit(&b).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded[1], b);
        assert_eq!(loaded[1].tag(), Some("holiday"));

        // An id that is not there changes nothing and is not an error, as upstream.
        let ghost = HistoryItem {
            id: 99,
            ..uploaded("ghost.png")
        };
        store.edit(&ghost).unwrap();
        assert_eq!(store.load().unwrap(), loaded);

        store.delete(&[items[0].clone(), items[2].clone()]).unwrap();
        store.delete(&[]).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].file_name, "b.png");
    }

    #[test]
    fn tags_are_newtonsoft_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE_NAME);
        let store = HistoryStore::open(&path).unwrap();
        let title = "Notes \"draft\" \\ ż\u{2028}\u{1}\t";
        let mut item = uploaded("t.png");
        let mut tags = HistoryTags::new();
        tags.insert("WindowTitle", Some(title.to_string()));
        tags.insert("ProcessName", Some("TextEdit".to_string()));
        item.tags = Some(tags);
        item.set_favorite(true);
        assert_eq!(item.tags_window_title(), Some(title));
        assert_eq!(item.tags_process_name(), Some("TextEdit"));
        assert!(item.favorite());
        store.append_item(&mut item).unwrap();
        assert_eq!(
            cells(&raw(&path), item.id)[9].as_deref(),
            Some(
                r#"{"WindowTitle":"Notes \"draft\" \\ ż\u2028\u0001\t","ProcessName":"TextEdit","Favorite":null}"#
            )
        );
        assert_eq!(store.load().unwrap()[0], item);

        // Unfavouriting keeps a dictionary, as upstream's setter does: `{}`, not NULL.
        let mut plain = uploaded("p.png");
        plain.set_favorite(false);
        assert_eq!(plain.tags, Some(HistoryTags::new()));
        store.append_item(&mut plain).unwrap();
        assert_eq!(cells(&raw(&path), plain.id)[9].as_deref(), Some("{}"));
        assert_eq!(store.load().unwrap()[1].tags, Some(HistoryTags::new()));

        // An empty tag removes the key; setting it again puts it last.
        item.set_tag("x");
        item.set_tag("");
        assert_eq!(item.tag(), None);
        item.set_tag("y");
        let last = item.tags.as_ref().unwrap().iter().last();
        assert_eq!(last, Some(("Tag", Some("y"))));

        // What Newtonsoft accepts besides strings, and what it does not.
        let read = HistoryTags::from_json(r#" {"n": 12, "b": true, "s": "x", "z": null} "#)
            .unwrap()
            .unwrap();
        assert_eq!(
            read.iter().collect::<Vec<_>>(),
            [
                ("n", Some("12")),
                ("b", Some("true")),
                ("s", Some("x")),
                ("z", None)
            ]
        );
        assert_eq!(HistoryTags::from_json("null").unwrap(), None);
        assert_eq!(HistoryTags::from_json("").unwrap(), None);
        assert!(HistoryTags::from_json(r#"{"a":{"b":1}}"#).is_err());
        assert!(HistoryTags::from_json("[1]").is_err());

        // A broken Tags cell makes the history unreadable, as it does upstream.
        raw(&path)
            .execute("UPDATE History SET Tags = '{oops' WHERE Id = 1", [])
            .unwrap();
        let error = store.load().unwrap_err();
        assert!(error.to_string().starts_with("row 1, Tags: "), "{error}");
    }

    #[test]
    fn date_times_are_written_as_dotnet_round_trip_local_times() {
        let instant = at("2026-07-01T10:20:30.1234567Z");
        let text = dotnet_text(instant).unwrap();
        let wall = instant.with_timezone(&Local);
        let expected = format!(
            "{}.1234567{}",
            wall.format("%Y-%m-%dT%H:%M:%S"),
            wall.format("%:z")
        );
        assert_eq!(text, expected);
        assert_eq!(text.len(), "2026-10-19T22:30:12.1234567+02:00".len());
        assert_eq!(parse_date_time(&text).unwrap(), Some(instant));

        // Ticks are 100 ns: anything finer does not survive, exactly as in .NET.
        let fine = at("2026-07-01T10:20:30.123456789Z");
        let back = parse_date_time(&dotnet_text(fine).unwrap()).unwrap();
        assert_eq!(back, Some(at("2026-07-01T10:20:30.1234567Z")));

        // Times a .NET DateTime cannot hold are not written.
        assert_eq!(
            dotnet_text(Utc.with_ymd_and_hms(10001, 1, 1, 0, 0, 0).unwrap()),
            None
        );
        let item = HistoryItem {
            date_time: Some(Utc.with_ymd_and_hms(-5, 1, 1, 0, 0, 0).unwrap()),
            ..uploaded("old.png")
        };
        assert!(date_time_cell(&item).is_err());
        let unset = HistoryItem::default();
        assert_eq!(date_time_cell(&unset).unwrap(), MIN_VALUE_TEXT);
    }

    #[test]
    fn date_times_are_read_like_dotnet_parse() {
        let instant = |text: &str| parse_date_time(text).unwrap().unwrap();
        // What upstream writes on a Windows machine in Warsaw, read anywhere.
        assert_eq!(
            instant("2026-10-19T22:30:12.1234567+02:00"),
            at("2026-10-19T20:30:12.1234567Z")
        );
        assert_eq!(
            instant("2026-01-15T22:30:12.1234567-05:30"),
            at("2026-01-16T04:00:12.1234567Z")
        );
        // A UTC DateTime (from the JSON migration) and Newtonsoft's trimmed fractions.
        assert_eq!(
            instant("2026-10-19T20:30:12.1234567Z"),
            at("2026-10-19T20:30:12.1234567Z")
        );
        assert_eq!(
            instant("2026-10-19T20:30:12.12Z"),
            at("2026-10-19T20:30:12.12Z")
        );
        assert_eq!(instant("2026-10-19T20:30:12Z"), at("2026-10-19T20:30:12Z"));
        assert_eq!(
            instant(" 2026-10-19 20:30:12.123456789+0000 "),
            at("2026-10-19T20:30:12.1234568Z")
        );
        // No zone is a local time.
        assert_eq!(
            instant("2026-07-01T09:30:00.0000000"),
            local(2026, 7, 1, 9, 30)
        );
        assert_eq!(instant("2026-01-15T09:30"), local(2026, 1, 15, 9, 30));
        assert_eq!(instant("2026-01-15"), local(2026, 1, 15, 0, 0));
        // DateTime.MinValue is "no time"; with a zone it is a time like any other.
        assert_eq!(parse_date_time(MIN_VALUE_TEXT).unwrap(), None);
        assert_eq!(parse_date_time("0001-01-01T00:00:00").unwrap(), None);
        assert!(parse_date_time("0001-01-01T00:00:00Z").unwrap().is_some());

        let refused = [
            "",
            "yesterday",
            "2026-13-01T00:00:00",
            "2026-02-30T00:00:00",
            "2026-10-19T24:00:00",
            "2026-10-19T10:00:00+15:00",
            "2026-10-19T10:00:00.Z",
            "2026-10-19T10:00:00Zjunk",
        ];
        for text in refused {
            let error = parse_date_time(text).unwrap_err();
            assert!(
                error.ends_with("was not recognized as a valid DateTime."),
                "{text}: {error}"
            );
        }
    }

    #[test]
    fn a_row_written_by_the_windows_app_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE_NAME);
        // The table as upstream creates it, and a row as upstream writes it.
        let windows = raw(&path);
        windows.execute_batch(CREATE_TABLE).unwrap();
        windows
            .execute(
                "INSERT INTO History (FileName, FilePath, DateTime, Type, Host, URL, ThumbnailURL, \
                 DeletionURL, ShortenedURL, Tags) VALUES ('Screenshot.png', \
                 'C:\\Users\\Ola\\Documents\\ShareX\\Screenshots\\2026-10\\Screenshot.png', \
                 '2026-10-19T22:30:12.1234567+02:00', 'Image', 'Modlight', 'https://example.com/v/abc', \
                 'https://example.com/r/abc', NULL, NULL, \
                 '{\"WindowTitle\":\"Pulpit\",\"ProcessName\":\"explorer\",\"Favorite\":null}')",
                [],
            )
            .unwrap();
        drop(windows);

        let store = HistoryStore::open(&path).unwrap();
        let items = store.load().unwrap();
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, 1);
        assert_eq!(item.date_time, Some(at("2026-10-19T20:30:12.1234567Z")));
        assert_eq!(
            item.file_path,
            "C:\\Users\\Ola\\Documents\\ShareX\\Screenshots\\2026-10\\Screenshot.png"
        );
        assert_eq!(item.deletion_url, "");
        assert_eq!(item.tags_window_title(), Some("Pulpit"));
        assert_eq!(item.tags_process_name(), Some("explorer"));
        assert!(item.favorite());
        assert!(item.is_valid());

        // The schema SQLite keeps is the text upstream's statement leaves there.
        let sql: String = raw(&path)
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'History'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            sql.starts_with(
                "CREATE TABLE History (\r\n    Id INTEGER PRIMARY KEY AUTOINCREMENT,\r\n"
            ),
            "{sql:?}"
        );
    }

    #[test]
    fn a_second_connection_sees_what_the_first_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE_NAME);
        let writer = HistoryStore::open(&path).unwrap();
        let reader = HistoryStore::open(&path).unwrap();
        assert!(reader.load().unwrap().is_empty());
        let mut item = uploaded("shared.png");
        writer.append_item(&mut item).unwrap();
        assert_eq!(reader.load().unwrap(), vec![item.clone()]);
        reader.delete(&[item]).unwrap();
        assert!(writer.load().unwrap().is_empty());
    }

    #[test]
    fn the_store_is_shared_between_threads() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::open(dir.path().join(HISTORY_FILE_NAME)).unwrap();
        let store = std::sync::Arc::new(store);
        let threads: Vec<_> = (0..4)
            .map(|thread| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for n in 0..10 {
                        let mut item = uploaded(&format!("{thread}-{n}.png"));
                        assert!(store.append_item(&mut item).unwrap());
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let mut ids: Vec<i64> = store.load().unwrap().iter().map(|i| i.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, (1..=40).collect::<Vec<_>>());
    }

    #[test]
    fn a_file_that_is_not_a_database_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE_NAME);
        std::fs::write(&path, vec![0x5a; 8192]).unwrap();
        let error = HistoryStore::open(&path).unwrap_err();
        assert!(matches!(error, HistoryError::Sqlite(_)), "{error}");

        // A folder where the file should be.
        let folder = dir.path().join("folder.db");
        std::fs::create_dir(&folder).unwrap();
        assert!(HistoryStore::open(&folder).is_err());
    }

    #[test]
    fn a_locked_file_is_an_error_after_the_busy_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE_NAME);
        let store = HistoryStore::open(&path).unwrap();
        let mut item = uploaded("before.png");
        store.append_item(&mut item).unwrap();

        let other = raw(&path);
        other.execute_batch("BEGIN EXCLUSIVE;").unwrap();
        let started = std::time::Instant::now();
        let error = store.append_item(&mut uploaded("blocked.png")).unwrap_err();
        let waited = started.elapsed();
        let busy = matches!(&error, HistoryError::Sqlite(e)
            if e.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy));
        assert!(busy, "{error}");
        assert!(
            waited >= std::time::Duration::from_millis(4500),
            "{waited:?}"
        );
        other.execute_batch("ROLLBACK;").unwrap();

        // Nothing was half-written, and the store works again.
        assert_eq!(store.load().unwrap(), vec![item]);
        assert!(store.append_item(&mut uploaded("after.png")).unwrap());
    }

    #[test]
    fn the_old_json_history_migrates_without_filtering() {
        let dir = tempfile::tempdir().unwrap();
        let json = dir.path().join(HISTORY_FILE_NAME_OLD);
        // As HistoryManagerJSON appends: indented objects joined by ",\r\n", null and
        // default members left out, a BOM in front.
        let text = "\u{feff}{\r\n  \"FileName\": \"old.png\",\r\n  \"FilePath\": \"C:\\\\old.png\",\r\n  \
                    \"DateTime\": \"2019-05-04T10:20:30.12+02:00\",\r\n  \"Type\": \"Image\",\r\n  \
                    \"Host\": \"Imgur\",\r\n  \"URL\": \"https://i.example.com/old.png\",\r\n  \
                    \"Tags\": {\r\n    \"Favorite\": null\r\n  }\r\n},\r\n{\r\n  \
                    \"FileName\": \"no-time.txt\",\r\n  \"Type\": \"Text\"\r\n}";
        std::fs::write(&json, text).unwrap();
        let items = read_json_history(&json).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].file_path, "C:\\old.png");
        assert_eq!(items[0].date_time, Some(at("2019-05-04T08:20:30.12Z")));
        assert!(items[0].favorite());
        assert_eq!(items[1].date_time, None);

        let path = dir.path().join(HISTORY_FILE_NAME);
        let store = HistoryStore::open(&path).unwrap();
        assert_eq!(store.migrate_from_json(&json).unwrap(), 2);
        let loaded = store.load().unwrap();
        assert_eq!(loaded[0].file_name, "old.png");
        assert_eq!(loaded[0].date_time, items[0].date_time);
        // The invalid one went in too, with MinValue's text, as upstream's migration does.
        assert_eq!(loaded[1].file_name, "no-time.txt");
        assert_eq!(loaded[1].date_time, None);
        let stored = cells(&raw(&path), loaded[1].id);
        assert_eq!(stored[2].as_deref(), Some(MIN_VALUE_TEXT));

        assert!(
            read_json_history(&dir.path().join("missing.json"))
                .unwrap()
                .is_empty()
        );
        std::fs::write(&json, "{\"FileName\": ").unwrap();
        assert!(matches!(
            read_json_history(&json),
            Err(HistoryError::Json(_))
        ));
    }
}
