//! Names made from patterns: ShareX's `NameParser` (file names, folder paths and
//! watermark texts such as `%y-%mo-%d_%ra{10}`), `UploadInfoParser` (the after-upload
//! texts with `$url`, `$filename`...), and the `FileHelpers` and `URLHelpers` pieces
//! they and the task flow use.
//!
//! The port follows .NET's behaviour on the system it runs on, where the two differ: on
//! Windows file names lose `"<>|:*?\/` and the control characters, `%n` is CRLF, `%un`
//! and `%uln` are the halves of the SAM account name and `%cn` the NetBIOS computer
//! name; on macOS file names lose NUL and `/` only, `%n` is "\n", `%un` is the login
//! name and `%cn` the host name up to its first dot. Upstream's .NET strings are UTF-16; lengths and
//! cuts count UTF-16 code units the same way. Where upstream throws, the functions
//! return a [`NameParserError`]; where upstream would hang or exhaust memory on an
//! absurd pattern, they stop with one too.

pub mod file_helpers;
pub mod helpers;
pub mod name_parser;
pub mod upload_info;
pub mod url_helpers;

pub use name_parser::{
    CODE_MENU_ENTRIES, CodeMenuEntry, CustomTimeZone, NameParser, NameParserError, NameParserType,
    machine_name, set_machine_name, user_domain_name, user_name,
};
pub use upload_info::{UploadInfo, UploadInfoParser, UploadLinks};
