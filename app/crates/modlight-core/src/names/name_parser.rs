//! `ShareX.HelpersLib/NameParser/NameParser.cs`: file names, folder paths and watermark
//! texts built from patterns such as `%y-%mo-%d_%h-%mi-%s`, with every code of
//! `CodeMenuEntryFilename.cs`.
//!
//! The codes are replaced in upstream's order, which matters: `%mon2` before `%mon`,
//! `%yy` before `%y`, `%width` before `%w`, `%unix` before `%un`, the `{n}` forms before
//! the plain ones, and the window title (`%t`) first, so codes in a title expand too.
//! Upstream's `string.IndexOf` (used by `ReplaceAll` and `ForEachBetween`) compares with
//! the current culture, which skips zero-width characters; here every search is
//! ordinal, the difference only shows with such characters inside a code.
//!
//! Where upstream's `Parse` throws (`%rf` outside preview mode, a negative padding, a
//! counter that wrapped), [`NameParser::parse`] returns a [`NameParserError`].

use std::fmt;
use std::sync::RwLock;

use chrono::{DateTime, Datelike, Local, NaiveDateTime, Timelike, Utc};
use rand::Rng;

use crate::culture::{Culture, CultureId};
use crate::l10n::{Key, helpers as strings, t};

use super::file_helpers::{is_text_file, path, sanitize_file_name, sanitize_path};
use super::helpers::{
    ALPHANUMERIC, ALPHANUMERIC_INVERSE, BASE56, MAX_GENERATED_LENGTH, MAX_REPLACEMENTS, NUMBERS,
    add_zeroes, add_zeroes_number, for_each_between, get_random_line_from_file, get_random_string,
    hour_to_12, random_adjective, random_animal, random_emoji, repeat_generator, replace_all,
    to_base, truncate, try_parse_int, week_of_year,
};
use super::url_helpers::get_valid_url;

/// `Helpers.Hexadecimal.ToLowerInvariant()`, what `%rx` draws from.
const HEXADECIMAL_LOWER: &str = "0123456789abcdef";
/// `Helpers.Hexadecimal.ToUpperInvariant()`, what `%rX` draws from.
const HEXADECIMAL_UPPER: &str = "0123456789ABCDEF";

/// `NameParserType`: what the result is for, which decides how it is cleaned up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NameParserType {
    #[default]
    Default,
    /// Free text (watermarks): `%n` is a new line.
    Text,
    /// A file name: `/` and NUL are removed, from the window title too.
    FileName,
    /// A folder path: NUL is removed; `/` stays, but not from the window title.
    FilePath,
    /// A URL: the path is percent-encoded (`Helpers.GetValidURL`).
    Url,
}

/// `NameParser.CustomTimeZone`.
///
/// Upstream accepts any `TimeZoneInfo`; this port knows UTC only, and any other zone
/// falls back to local time, the same as no custom zone at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CustomTimeZone {
    /// `TimeZoneInfo.Utc`, the default of upstream's "custom time zone" setting.
    Utc,
    /// Any other zone, by its upstream id; treated as local time.
    Other(String),
}

impl CustomTimeZone {
    /// The zone a `TimeZoneInfo.Id` stands for ("UTC" on Windows, "Etc/UTC" in IANA).
    pub fn from_id(id: &str) -> CustomTimeZone {
        if id.eq_ignore_ascii_case("UTC") || id.eq_ignore_ascii_case("Etc/UTC") {
            CustomTimeZone::Utc
        } else {
            CustomTimeZone::Other(id.to_string())
        }
    }
}

/// Why a pattern could not be parsed: the places where upstream's `Parse` throws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameParserError {
    /// `%rf{path}` outside preview mode named something that is not a text file or that
    /// could not be read. The text is upstream's exception message; in preview mode the
    /// message is written into the result instead.
    RandomLineFile(String),
    /// A number format .NET throws on, with .NET's message: a negative zero padding
    /// (`%ia{-1}`), a precision past 999,999,999 (`%i{1000000000}`), or a counter that
    /// wrapped to `int.MinValue` (the magnitude does not fit).
    InvalidNumberFormat(String),
    /// A repeat count or padding over [`MAX_GENERATED_LENGTH`]; upstream would run out of
    /// memory or time.
    TooLong(i64),
    /// The replacement of this code kept bringing the code back (more than
    /// [`MAX_REPLACEMENTS`] times); upstream loops forever.
    Endless(String),
}

impl fmt::Display for NameParserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NameParserError::RandomLineFile(message)
            | NameParserError::InvalidNumberFormat(message) => f.write_str(message),
            NameParserError::TooLong(length) => write!(
                f,
                "The pattern asks for {length} characters at once; the limit is {MAX_GENERATED_LENGTH}."
            ),
            NameParserError::Endless(code) => write!(
                f,
                "\"{code}\" was replaced {MAX_REPLACEMENTS} times and keeps coming back."
            ),
        }
    }
}

impl std::error::Error for NameParserError {}

/// One code of a pattern menu (`CodeMenuEntry`): its prefix (`%` for the name codes of
/// `CodeMenuEntryFilename`, `$` for the action and pixel codes upstream also has), the
/// code without it, what the menu says about it, and the menu group it sits in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeMenuEntry {
    pub prefix: &'static str,
    pub value: &'static str,
    pub description: Key,
    pub category: Option<Key>,
}

impl CodeMenuEntry {
    /// `ToPrefixString()`: the code as it is typed, `%y`.
    pub fn to_prefix_string(&self) -> String {
        self.with_prefix(self.prefix)
    }

    /// `ToPrefixString(prefix)`: the code behind another prefix.
    pub fn with_prefix(&self, prefix: &str) -> String {
        format!("{prefix}{}", self.value)
    }
}

/// A `CodeMenuEntryFilename`.
const fn entry(value: &'static str, description: Key, category: Option<Key>) -> CodeMenuEntry {
    CodeMenuEntry {
        prefix: "%",
        value,
        description,
        category,
    }
}

/// The fields of `CodeMenuEntryFilename`, one constant each.
pub mod codes {
    use super::{CodeMenuEntry, entry, strings};

    const WINDOW: Option<crate::l10n::Key> = Some(strings::ReplCodeMenuCategory_Window);
    const DATE: Option<crate::l10n::Key> = Some(strings::ReplCodeMenuCategory_Date_and_Time);
    const INCREMENTAL: Option<crate::l10n::Key> = Some(strings::ReplCodeMenuCategory_Incremental);
    const RANDOM: Option<crate::l10n::Key> = Some(strings::ReplCodeMenuCategory_Random);
    const IMAGE: Option<crate::l10n::Key> = Some(strings::ReplCodeMenuCategory_Image);
    const COMPUTER: Option<crate::l10n::Key> = Some(strings::ReplCodeMenuCategory_Computer);

    pub const T: CodeMenuEntry = entry(
        "t",
        strings::ReplCodeMenuEntry_t_Title_of_active_window,
        WINDOW,
    );
    pub const PN: CodeMenuEntry = entry(
        "pn",
        strings::ReplCodeMenuEntry_pn_Process_name_of_active_window,
        WINDOW,
    );
    pub const Y: CodeMenuEntry = entry("y", strings::ReplCodeMenuEntry_y_Current_year, DATE);
    pub const YY: CodeMenuEntry = entry(
        "yy",
        strings::ReplCodeMenuEntry_yy_Current_year__2_digits_,
        DATE,
    );
    pub const MO: CodeMenuEntry = entry("mo", strings::ReplCodeMenuEntry_mo_Current_month, DATE);
    pub const MON: CodeMenuEntry = entry(
        "mon",
        strings::ReplCodeMenuEntry_mon_Current_month_name__Local_language_,
        DATE,
    );
    pub const MON2: CodeMenuEntry = entry(
        "mon2",
        strings::ReplCodeMenuEntry_mon2_Current_month_name__English_,
        DATE,
    );
    pub const W: CodeMenuEntry = entry(
        "w",
        strings::ReplCodeMenuEntry_w_Current_week_name__Local_language_,
        DATE,
    );
    pub const W2: CodeMenuEntry = entry(
        "w2",
        strings::ReplCodeMenuEntry_w2_Current_week_name__English_,
        DATE,
    );
    pub const WY: CodeMenuEntry = entry("wy", strings::ReplCodeMenuEntry_wy_Week_of_year, DATE);
    pub const D: CodeMenuEntry = entry("d", strings::ReplCodeMenuEntry_d_Current_day, DATE);
    pub const H: CodeMenuEntry = entry("h", strings::ReplCodeMenuEntry_h_Current_hour, DATE);
    pub const MI: CodeMenuEntry = entry("mi", strings::ReplCodeMenuEntry_mi_Current_minute, DATE);
    pub const S: CodeMenuEntry = entry("s", strings::ReplCodeMenuEntry_s_Current_second, DATE);
    pub const MS: CodeMenuEntry = entry(
        "ms",
        strings::ReplCodeMenuEntry_ms_Current_millisecond,
        DATE,
    );
    pub const PM: CodeMenuEntry = entry("pm", strings::ReplCodeMenuEntry_pm_Gets_AM_PM, DATE);
    pub const UNIX: CodeMenuEntry =
        entry("unix", strings::ReplCodeMenuEntry_unix_Unix_timestamp, DATE);
    pub const I: CodeMenuEntry = entry(
        "i",
        strings::ReplCodeMenuEntry_i_Auto_increment_number,
        INCREMENTAL,
    );
    pub const IA: CodeMenuEntry = entry(
        "ia",
        strings::ReplCodeMenuEntry_ia_Auto_increment_alphanumeric,
        INCREMENTAL,
    );
    pub const I_AA: CodeMenuEntry = entry(
        "iAa",
        strings::ReplCodeMenuEntry_iAa_Auto_increment_alphanumeric_all,
        INCREMENTAL,
    );
    pub const IB: CodeMenuEntry = entry(
        "ib",
        strings::ReplCodeMenuEntry_ib_Auto_increment_base_alphanumeric,
        INCREMENTAL,
    );
    pub const IX: CodeMenuEntry = entry(
        "ix",
        strings::ReplCodeMenuEntry_ix_Auto_increment_hexadecimal,
        INCREMENTAL,
    );
    pub const RN: CodeMenuEntry = entry(
        "rn",
        strings::ReplCodeMenuEntry_rn_Random_number_0_to_9,
        RANDOM,
    );
    pub const RA: CodeMenuEntry = entry(
        "ra",
        strings::ReplCodeMenuEntry_ra_Random_alphanumeric_char,
        RANDOM,
    );
    pub const RNA: CodeMenuEntry = entry(
        "rna",
        strings::RandomNonAmbiguousAlphanumericCharRepeatUsingN,
        RANDOM,
    );
    pub const RX: CodeMenuEntry = entry(
        "rx",
        strings::ReplCodeMenuEntry_rx_Random_hexadecimal,
        RANDOM,
    );
    pub const GUID: CodeMenuEntry =
        entry("guid", strings::ReplCodeMenuEntry_guid_Random_guid, RANDOM);
    pub const RADJECTIVE: CodeMenuEntry = entry(
        "radjective",
        strings::CodeMenuEntryFilename_RandomAdjective,
        RANDOM,
    );
    pub const RANIMAL: CodeMenuEntry = entry(
        "ranimal",
        strings::CodeMenuEntryFilename_RandomAnimal,
        RANDOM,
    );
    pub const REMOJI: CodeMenuEntry = entry("remoji", strings::RandomEmojiRepeatUsingN, RANDOM);
    pub const RF: CodeMenuEntry = entry(
        "rf",
        strings::ReplCodeMenuEntry_rf_Random_line_from_file,
        RANDOM,
    );
    pub const WIDTH: CodeMenuEntry = entry(
        "width",
        strings::ReplCodeMenuEntry_width_Gets_image_width,
        IMAGE,
    );
    pub const HEIGHT: CodeMenuEntry = entry(
        "height",
        strings::ReplCodeMenuEntry_height_Gets_image_height,
        IMAGE,
    );
    pub const UN: CodeMenuEntry = entry("un", strings::ReplCodeMenuEntry_un_User_name, COMPUTER);
    pub const ULN: CodeMenuEntry = entry(
        "uln",
        strings::ReplCodeMenuEntry_uln_User_login_name,
        COMPUTER,
    );
    pub const CN: CodeMenuEntry =
        entry("cn", strings::ReplCodeMenuEntry_cn_Computer_name, COMPUTER);
    pub const N: CodeMenuEntry = entry("n", strings::ReplCodeMenuEntry_n_New_line, None);
}

/// Every code, in the order upstream's code menu lists them (declaration order).
pub const CODE_MENU_ENTRIES: [CodeMenuEntry; 37] = [
    codes::T,
    codes::PN,
    codes::Y,
    codes::YY,
    codes::MO,
    codes::MON,
    codes::MON2,
    codes::W,
    codes::W2,
    codes::WY,
    codes::D,
    codes::H,
    codes::MI,
    codes::S,
    codes::MS,
    codes::PM,
    codes::UNIX,
    codes::I,
    codes::IA,
    codes::I_AA,
    codes::IB,
    codes::IX,
    codes::RN,
    codes::RA,
    codes::RNA,
    codes::RX,
    codes::GUID,
    codes::RADJECTIVE,
    codes::RANIMAL,
    codes::REMOJI,
    codes::RF,
    codes::WIDTH,
    codes::HEIGHT,
    codes::UN,
    codes::ULN,
    codes::CN,
    codes::N,
];

/// `CultureInfo.InvariantCulture.DateTimeFormat.GetMonthName` (`%mon2`).
const INVARIANT_MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
/// `GetDayName` of the invariant culture (`%w2`), in .NET's `DayOfWeek` order.
const INVARIANT_DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
/// pl-PL `GetMonthName`: the nominative, not the genitive the long date pattern uses.
const POLISH_MONTHS: [&str; 12] = [
    "styczeń",
    "luty",
    "marzec",
    "kwiecień",
    "maj",
    "czerwiec",
    "lipiec",
    "sierpień",
    "wrzesień",
    "październik",
    "listopad",
    "grudzień",
];
/// pl-PL `GetDayName`, in `DayOfWeek` order.
const POLISH_DAYS: [&str; 7] = [
    "niedziela",
    "poniedziałek",
    "wtorek",
    "środa",
    "czwartek",
    "piątek",
    "sobota",
];

/// `%mon`: the month in the current culture (`CultureInfo.CurrentCulture`).
pub fn month_name(culture: Culture, month0: usize) -> &'static str {
    let names = match culture.id {
        CultureId::EnglishUs => &INVARIANT_MONTHS,
        CultureId::PolishPl => &POLISH_MONTHS,
    };
    names.get(month0).copied().unwrap_or_default()
}

/// `%w`: the day of the week in the current culture.
pub fn day_name(culture: Culture, day_from_sunday: usize) -> &'static str {
    let names = match culture.id {
        CultureId::EnglishUs => &INVARIANT_DAYS,
        CultureId::PolishPl => &POLISH_DAYS,
    };
    names.get(day_from_sunday).copied().unwrap_or_default()
}

static MACHINE_NAME: RwLock<Option<String>> = RwLock::new(None);

/// Tell the parsers the computer's name, as `hostname` gives it. Like .NET's
/// `Environment.MachineName` on macOS only the part before the first dot is kept
/// ("Marcins-MacBook-Pro.local" is "Marcins-MacBook-Pro"). On macOS the app calls this
/// at start (this crate cannot ask the system there); until then `%cn` and `%uln` are
/// "localhost". On Windows the name comes from the system by itself.
pub fn set_machine_name(host_name: &str) {
    let name = host_name.split('.').next().unwrap_or_default().to_string();
    if let Ok(mut slot) = MACHINE_NAME.write() {
        *slot = Some(name);
    }
}

/// `Environment.MachineName` (`%cn`).
pub fn machine_name() -> String {
    MACHINE_NAME
        .read()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or_else(system_machine_name)
}

#[cfg(not(windows))]
fn system_machine_name() -> String {
    "localhost".to_string()
}

/// `GetComputerNameW`, the NetBIOS name .NET's `MachineName` answers on Windows.
#[cfg(windows)]
fn system_machine_name() -> String {
    use windows_sys::Win32::System::WindowsProgramming::GetComputerNameW;
    let mut buffer = [0u16; 64];
    let mut length = buffer.len() as u32;
    // SAFETY: the buffer's length is passed with it and updated by the call.
    if unsafe { GetComputerNameW(buffer.as_mut_ptr(), &mut length) } != 0 {
        String::from_utf16_lossy(&buffer[..length as usize])
    } else {
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".to_string())
    }
}

/// `Environment.UserName` (`%un`): the login name, from `$USER` (`$LOGNAME` when unset).
#[cfg(not(windows))]
pub fn user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default()
}

/// `Environment.UserDomainName` (`%uln`): .NET on macOS answers the machine name.
#[cfg(not(windows))]
pub fn user_domain_name() -> String {
    machine_name()
}

/// `Environment.UserName` (`%un`) on Windows: the user part of the SAM name
/// (`GetUserNameExW(NameSamCompatible)`, "DOMAIN\user"), as .NET reads it.
#[cfg(windows)]
pub fn user_name() -> String {
    match sam_compatible_name() {
        Some(name) => match name.split_once('\\') {
            Some((_, user)) => user.to_string(),
            None => name,
        },
        None => std::env::var("USERNAME").unwrap_or_default(),
    }
}

/// `Environment.UserDomainName` (`%uln`) on Windows: the domain part of the SAM name,
/// which for a local or Microsoft account is the computer's name.
#[cfg(windows)]
pub fn user_domain_name() -> String {
    sam_compatible_name()
        .and_then(|name| name.split_once('\\').map(|(domain, _)| domain.to_string()))
        .or_else(|| std::env::var("USERDOMAIN").ok())
        .unwrap_or_else(machine_name)
}

#[cfg(windows)]
fn sam_compatible_name() -> Option<String> {
    use windows_sys::Win32::Security::Authentication::Identity::{
        GetUserNameExW, NameSamCompatible,
    };
    let mut buffer = vec![0u16; 64];
    loop {
        let mut size = buffer.len() as u32;
        // SAFETY: the buffer's length is passed with it; on success `size` is the
        // length without the terminator, on ERROR_MORE_DATA the length needed.
        if unsafe { GetUserNameExW(NameSamCompatible, buffer.as_mut_ptr(), &mut size) } {
            return Some(String::from_utf16_lossy(&buffer[..size as usize]));
        }
        if size as usize <= buffer.len() || size > 4096 {
            return None;
        }
        buffer.resize(size as usize, 0);
    }
}

/// `Environment.NewLine`: what `%n` becomes in free text.
const NEW_LINE: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// `NameParser.Parse(NameParserType, pattern)`: one parse with the default settings.
pub fn parse(parser_type: NameParserType, pattern: &str) -> Result<String, NameParserError> {
    NameParser::new(parser_type).parse(pattern)
}

/// `NameParser`: the settings of one parse. The counter (`auto_increment_number`) goes
/// up by one in every [`parse`](NameParser::parse) whose pattern holds `%i`; callers
/// keep it in their settings like upstream keeps `NameParserAutoIncrementNumber`.
#[derive(Clone, Debug, PartialEq)]
pub struct NameParser {
    parser_type: NameParserType,
    /// `MaxNameLength`: the result is cut to this many UTF-16 units (0: no limit).
    pub max_name_length: i32,
    /// `MaxTitleLength`: the window title is cut to this many units (0: no limit).
    pub max_title_length: i32,
    /// `AutoIncrementNumber`, read and written by `%i`, `%ia`, `%iAa`, `%ib`, `%ix`.
    pub auto_increment_number: i32,
    /// `ImageWidth` (`%width`); 0 or less writes nothing.
    pub image_width: i32,
    /// `ImageHeight` (`%height`); 0 or less writes nothing.
    pub image_height: i32,
    /// `WindowText` (`%t`); with `None` the code stays as typed.
    pub window_text: Option<String>,
    /// `ProcessName` (`%pn`); with `None` the code stays as typed.
    pub process_name: Option<String>,
    /// `CustomTimeZone`: `None` is local time.
    pub custom_time_zone: Option<CustomTimeZone>,
    /// `AllowFileRead`: whether `%rf{path}` may read files; when not, the code is removed.
    pub allow_file_read: bool,
    /// `IsPreviewMode`: a failing `%rf` writes its error into the result instead of
    /// failing the parse (the settings preview).
    pub is_preview_mode: bool,
    /// The moment the date codes and `%unix` describe; `None` is the clock. Tests set it.
    pub now: Option<DateTime<Utc>>,
}

impl NameParser {
    pub fn new(parser_type: NameParserType) -> NameParser {
        NameParser {
            parser_type,
            max_name_length: 0,
            max_title_length: 0,
            auto_increment_number: 0,
            image_width: 0,
            image_height: 0,
            window_text: None,
            process_name: None,
            custom_time_zone: None,
            allow_file_read: true,
            is_preview_mode: false,
            now: None,
        }
    }

    /// `Type`, fixed at construction like upstream's private setter.
    pub fn parser_type(&self) -> NameParserType {
        self.parser_type
    }

    /// `Parse(pattern)`.
    pub fn parse(&mut self, pattern: &str) -> Result<String, NameParserError> {
        if pattern.is_empty() {
            return Ok(String::new());
        }

        let mut sb = pattern.to_string();

        if let Some(window_text) = &self.window_text {
            let mut window_text = self.sanitize_input(window_text);
            if self.max_title_length > 0 {
                window_text = truncate(&window_text, self.max_title_length);
            }
            sb = sb.replace("%t", &window_text);
        }

        if let Some(process_name) = &self.process_name {
            sb = sb.replace("%pn", &self.sanitize_input(process_name));
        }

        let width = positive_or_empty(self.image_width);
        let height = positive_or_empty(self.image_height);
        sb = sb.replace("%width", &width).replace("%height", &height);

        let now = self.now.unwrap_or_else(Utc::now);
        let dt = self.date_time(now);
        let culture = Culture::current();
        let month0 = dt.month0() as usize;
        sb = sb
            .replace("%mon2", INVARIANT_MONTHS[month0])
            .replace("%mon", month_name(culture, month0))
            .replace(
                "%yy",
                &add_zeroes_number(i64::from(dt.year().rem_euclid(100)), 2),
            )
            .replace("%y", &dt.year().to_string())
            .replace("%mo", &add_zeroes_number(i64::from(dt.month()), 2))
            .replace("%d", &add_zeroes_number(i64::from(dt.day()), 2));

        let hour = if sb.contains("%pm") {
            hour_to_12(dt.hour())
        } else {
            add_zeroes_number(i64::from(dt.hour()), 2)
        };
        // chrono keeps a leap second in the nanoseconds; .NET's milliseconds stop at 999.
        let millisecond = (dt.nanosecond() / 1_000_000).min(999);
        let day = dt.weekday().num_days_from_sunday() as usize;
        sb = sb
            .replace("%h", &hour)
            .replace("%mi", &add_zeroes_number(i64::from(dt.minute()), 2))
            .replace("%s", &add_zeroes_number(i64::from(dt.second()), 2))
            .replace("%ms", &add_zeroes_number(i64::from(millisecond), 3))
            .replace("%wy", &week_of_year(&dt).to_string())
            .replace("%w2", INVARIANT_DAYS[day])
            .replace("%w", day_name(culture, day))
            .replace("%pm", if dt.hour() >= 12 { "PM" } else { "AM" });

        sb = sb.replace("%unix", &now.timestamp().to_string());

        // Every counter code starts with "%i", so this is upstream's long condition.
        if sb.contains("%i") {
            // C# `int` arithmetic is unchecked: int.MaxValue + 1 wraps around.
            self.auto_increment_number = self.auto_increment_number.wrapping_add(1);
            replace_counters(&mut sb, self.auto_increment_number)?;
        }

        sb = sb
            .replace("%un", &user_name())
            .replace("%uln", &user_domain_name())
            .replace("%cn", &machine_name());

        if self.parser_type == NameParserType::Text {
            sb = sb.replace("%n", NEW_LINE);
        }

        let mut result = sb;

        result = replace_all(&result, "%radjective", || Ok(random_adjective()))?;
        result = replace_all(&result, "%ranimal", || Ok(random_animal()))?;

        for (token, file_path) in entries_with_argument(&result, "%rf") {
            if !self.allow_file_read {
                result = result.replace(&token, "");
                continue;
            }
            result = replace_all(&result, &token, || self.random_line_of(&file_path))?;
        }

        for (code, chars) in [
            ("%rna", BASE56),
            ("%rn", NUMBERS),
            ("%ra", ALPHANUMERIC),
            ("%rx", HEXADECIMAL_LOWER),
            ("%rX", HEXADECIMAL_UPPER),
        ] {
            for (token, count) in entries_with_values::<1>(&result, code) {
                let count = count[0];
                check_length(i64::from(count))?;
                result = replace_all(&result, &token, || {
                    Ok(repeat_generator(count, || random_char(chars)))
                })?;
            }
        }

        for (token, count) in entries_with_values::<1>(&result, "%remoji") {
            let count = count[0];
            check_length(i64::from(count))?;
            result = replace_all(&result, &token, || {
                Ok(repeat_generator(count, random_emoji))
            })?;
        }

        for (code, chars) in [
            ("%rna", BASE56),
            ("%rn", NUMBERS),
            ("%ra", ALPHANUMERIC),
            ("%rx", HEXADECIMAL_LOWER),
            ("%rX", HEXADECIMAL_UPPER),
        ] {
            result = replace_all(&result, code, || Ok(random_char(chars)))?;
        }
        result = replace_all(&result, "%guid", || Ok(new_guid()))?;
        result = replace_all(&result, "%GUID", || Ok(new_guid().to_uppercase()))?;
        result = replace_all(&result, "%remoji", || Ok(random_emoji()))?;

        result = match self.parser_type {
            NameParserType::FileName => sanitize_file_name(&result),
            NameParserType::FilePath => sanitize_path(&result),
            NameParserType::Url => get_valid_url(&result, false),
            NameParserType::Default | NameParserType::Text => result,
        };

        if self.max_name_length > 0 {
            result = truncate(&result, self.max_name_length);
        }

        Ok(result)
    }

    /// `DateTime.Now`, converted to the custom zone when there is one.
    fn date_time(&self, now: DateTime<Utc>) -> NaiveDateTime {
        match self.custom_time_zone {
            Some(CustomTimeZone::Utc) => now.naive_utc(),
            Some(CustomTimeZone::Other(_)) | None => now.with_timezone(&Local).naive_local(),
        }
    }

    /// `SanitizeInput`: the window title and process name, trimmed, spaces as `_`, and
    /// for names and paths without the characters a file name cannot hold.
    fn sanitize_input(&self, input: &str) -> String {
        let input = input.trim().replace(' ', "_");
        match self.parser_type {
            NameParserType::FileName | NameParserType::FilePath => sanitize_file_name(&input),
            _ => input,
        }
    }

    /// One `%rf{path}`: a random line of a text file, or what went wrong.
    fn random_line_of(&self, file_path: &str) -> Result<String, NameParserError> {
        let outcome = if is_text_file(file_path) {
            get_random_line_from_file(file_path)
                .map_err(|error| io_error_message(&error, file_path))
        } else {
            Err(t(strings::NameParser_Valid_text_file_required).to_string())
        };
        match outcome {
            Ok(line) => Ok(line),
            Err(message) if self.is_preview_mode => Ok(message),
            Err(message) => Err(NameParserError::RandomLineFile(message)),
        }
    }
}

fn positive_or_empty(value: i32) -> String {
    if value > 0 {
        value.to_string()
    } else {
        String::new()
    }
}

fn random_char(chars: &str) -> String {
    get_random_string(chars, 1)
}

/// `Guid.NewGuid().ToString()`: a random version 4 GUID, "D" format, small letters.
fn new_guid() -> String {
    let mut bytes = [0u8; 16];
    super::helpers::with_rng(|rng| rng.fill(&mut bytes));
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let hex = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The message of the .NET exception `File.ReadAllText` throws for this error, with the
/// full path in it the way .NET writes it.
fn io_error_message(error: &std::io::Error, file_path: &str) -> String {
    use std::io::ErrorKind;
    let full = path::get_full_path(file_path);
    match error.kind() {
        ErrorKind::NotFound => {
            // .NET blames the folder when the folder is missing too.
            let folder_exists = path::get_directory_name(&full)
                .is_some_and(|folder| std::path::Path::new(&folder).is_dir());
            if folder_exists {
                format!("Could not find file '{full}'.")
            } else {
                format!("Could not find a part of the path '{full}'.")
            }
        }
        // .NET opens a folder and then refuses it as access denied.
        ErrorKind::PermissionDenied | ErrorKind::IsADirectory => {
            format!("Access to the path '{full}' is denied.")
        }
        ErrorKind::InvalidFilename => format!(
            "The path '{full}' is too long, or a component of the specified path is too long."
        ),
        _ => {
            let text = error.to_string();
            let text = match text.rfind(" (os error ") {
                Some(at) => text[..at].to_string(),
                None => text,
            };
            format!("{text} : '{full}'")
        }
    }
}

/// Upstream's guard against absurd counts is running out of memory; ours is a limit.
fn check_length(length: i64) -> Result<(), NameParserError> {
    if length > MAX_GENERATED_LENGTH {
        return Err(NameParserError::TooLong(length));
    }
    Ok(())
}

/// `Helpers.AddZeroes(text, width)` where the width comes from the pattern: `PadLeft`
/// throws on a negative width.
fn pad_zeroes(input: &str, width: i32) -> Result<String, NameParserError> {
    if width < 0 {
        return Err(NameParserError::InvalidNumberFormat(format!(
            "totalWidth ('{width}') must be a non-negative value. (Parameter 'totalWidth')\nActual value was {width}."
        )));
    }
    check_length(i64::from(width))?;
    Ok(add_zeroes(input, width as usize))
}

/// `ListEntryWithValues`: every `code{a,b,...}` with its first `N` numbers (missing or
/// unreadable ones are 0, like `int.TryParse` leaves them).
fn entries_with_values<const N: usize>(text: &str, code: &str) -> Vec<(String, [i32; N])> {
    for_each_between(text, &format!("{code}{{"), "}")
        .into_iter()
        .map(|(token, inner)| {
            let parts: Vec<&str> = inner.split(',').collect();
            let mut values = [0; N];
            for (i, value) in values.iter_mut().enumerate() {
                *value = parts
                    .get(i)
                    .and_then(|part| try_parse_int(part))
                    .unwrap_or(0);
            }
            (token, values)
        })
        .collect()
}

/// `ListEntryWithArgument`: every `code{text}` with the text up to its first comma.
fn entries_with_argument(text: &str, code: &str) -> Vec<(String, String)> {
    for_each_between(text, &format!("{code}{{"), "}")
        .into_iter()
        .map(|(token, inner)| {
            let first = inner.split(',').next().unwrap_or_default().to_string();
            (token, first)
        })
        .collect()
}

/// The counter codes, in upstream's order: `%ib{radix,width}` and `%iB{...}`, then
/// `%iAa`, `%iaA`, `%ia`, `%iA`, `%ix`, `%iX` and `%i`, each `{width}` form before the
/// plain one.
fn replace_counters(sb: &mut String, number: i32) -> Result<(), NameParserError> {
    // Upstream runs both base loops in one try block and swallows any failure: the
    // remaining base codes stay and `%i` below eats their "%i".
    let _ = replace_base_codes(sb, number);

    replace_to_base(sb, "%iAa", number, 62, ALPHANUMERIC, LetterCase::Keep)?;
    replace_to_base(
        sb,
        "%iaA",
        number,
        62,
        ALPHANUMERIC_INVERSE,
        LetterCase::Keep,
    )?;
    replace_to_base(sb, "%ia", number, 36, ALPHANUMERIC, LetterCase::Lower)?;
    replace_to_base(sb, "%iA", number, 36, ALPHANUMERIC, LetterCase::Upper)?;
    replace_formatted(sb, "%ix", number, 'x')?;
    replace_formatted(sb, "%iX", number, 'X')?;
    replace_formatted(sb, "%i", number, 'd')
}

fn replace_base_codes(sb: &mut String, number: i32) -> Result<(), NameParserError> {
    for (code, digits) in [("%ib", ALPHANUMERIC_INVERSE), ("%iB", ALPHANUMERIC)] {
        for (token, [radix, width]) in entries_with_values::<2>(sb, code) {
            let text = pad_zeroes(&to_base(number, radix, digits)?, width)?;
            *sb = sb.replace(&token, &text);
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum LetterCase {
    Keep,
    Lower,
    Upper,
}

impl LetterCase {
    fn apply(self, text: String) -> String {
        match self {
            LetterCase::Keep => text,
            LetterCase::Lower => text.to_lowercase(),
            LetterCase::Upper => text.to_uppercase(),
        }
    }
}

fn replace_to_base(
    sb: &mut String,
    code: &str,
    number: i32,
    radix: i32,
    digits: &str,
    case: LetterCase,
) -> Result<(), NameParserError> {
    for (token, [width]) in entries_with_values::<1>(sb, code) {
        let text = case.apply(pad_zeroes(&to_base(number, radix, digits)?, width)?);
        *sb = sb.replace(&token, &text);
    }
    // Upstream computes the plain replacement even when the code is absent, so a
    // counter at int.MinValue fails here whatever the pattern holds.
    let plain = case.apply(to_base(number, radix, digits)?);
    *sb = sb.replace(code, &plain);
    Ok(())
}

fn replace_formatted(
    sb: &mut String,
    code: &str,
    number: i32,
    letter: char,
) -> Result<(), NameParserError> {
    for (token, [precision]) in entries_with_values::<1>(sb, code) {
        let text = format_number(number, letter, Some(precision))?;
        *sb = sb.replace(&token, &text);
    }
    let plain = format_number(number, letter, None)?;
    *sb = sb.replace(code, &plain);
    Ok(())
}

/// `int.ToString(letter + precision)` for "d", "x" and "X".
///
/// A precision of 0 or more is the standard format (zero padded; hexadecimal shows
/// negative numbers in two's complement). A negative `{n}` makes upstream build "d-3",
/// which .NET reads as a custom format of literals: see [`custom_format`].
fn format_number(
    value: i32,
    letter: char,
    precision: Option<i32>,
) -> Result<String, NameParserError> {
    let precision = match precision {
        None => 0,
        Some(precision) if precision < 0 => {
            return Ok(custom_format(value, &format!("{letter}{precision}")));
        }
        Some(precision) if precision > 999_999_999 => {
            return Err(NameParserError::InvalidNumberFormat(
                "Format specifier was invalid.".to_string(),
            ));
        }
        Some(precision) => {
            check_length(i64::from(precision))?;
            precision as usize
        }
    };
    Ok(match letter {
        // The cast keeps the bits: .NET prints a negative int's two's complement.
        'x' => add_zeroes(&format!("{:x}", value as u32), precision),
        'X' => add_zeroes(&format!("{:X}", value as u32), precision),
        _ => {
            let digits = add_zeroes(&i64::from(value).unsigned_abs().to_string(), precision);
            if value < 0 {
                format!("-{digits}")
            } else {
                digits
            }
        }
    })
}

/// .NET's custom numeric format, for the formats [`format_number`] can produce: literal
/// characters and `0` digit placeholders ("x-10" is the literals "x-1" and one
/// placeholder). The digits fill the placeholders from the right, any extra ones go to
/// the first placeholder; with no placeholder only the literals are written.
fn custom_format(value: i32, format: &str) -> String {
    let placeholders = format.chars().filter(|c| *c == '0').count();
    let mut out = String::new();
    if value < 0 {
        out.push('-');
    }
    if placeholders == 0 {
        out.push_str(format);
        return out;
    }
    let digits = if value == 0 {
        String::new()
    } else {
        i64::from(value).unsigned_abs().to_string()
    };
    let digits: Vec<char> = add_zeroes(&digits, placeholders).chars().collect();
    let mut next = 0;
    let mut first = true;
    for c in format.chars() {
        if c != '0' {
            out.push(c);
            continue;
        }
        let take = if first {
            digits.len() + 1 - placeholders
        } else {
            1
        };
        first = false;
        out.extend(digits.iter().skip(next).take(take));
        next += take;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::culture::set_thread_culture;
    use crate::l10n::{Language, set_thread_language};
    use chrono::TimeZone;

    /// Monday 2026-10-19 14:05:09.042 UTC.
    fn monday_afternoon() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 19, 14, 5, 9).unwrap() + chrono::Duration::milliseconds(42)
    }

    fn parser(parser_type: NameParserType) -> NameParser {
        let mut parser = NameParser::new(parser_type);
        parser.now = Some(monday_afternoon());
        parser.custom_time_zone = Some(CustomTimeZone::Utc);
        parser
    }

    fn parse_at(parser_type: NameParserType, pattern: &str) -> String {
        parser(parser_type).parse(pattern).unwrap()
    }

    #[test]
    fn date_and_time_codes() {
        set_thread_culture(Some(Culture::english()));
        let p = |pattern| parse_at(NameParserType::Default, pattern);
        assert_eq!(p("%y-%mo-%d_%h-%mi-%s_%ms"), "2026-10-19_14-05-09_042");
        assert_eq!(p("%yy"), "26");
        assert_eq!(p("%mon %mon2 %w %w2"), "October October Monday Monday");
        assert_eq!(p("%wy"), "43");
        assert_eq!(p("%h:%mi %pm"), "02:05 PM");
        assert_eq!(p("%pm%h"), "PM02");
        assert_eq!(p("%unix"), "1792418709");
        assert_eq!(p("%d%d"), "1919");
        set_thread_culture(Some(Culture::polish()));
        assert_eq!(
            p("%mon %mon2 %w %w2"),
            "październik October poniedziałek Monday"
        );
        set_thread_culture(None);
    }

    #[test]
    fn hours_on_the_twelve_hour_clock() {
        let at = |hour| {
            let mut parser = parser(NameParserType::Default);
            parser.now = Some(Utc.with_ymd_and_hms(2026, 1, 1, hour, 0, 0).unwrap());
            (
                parser.clone().parse("%h%pm").unwrap(),
                parser.parse("%h").unwrap(),
            )
        };
        assert_eq!(at(0), ("12AM".to_string(), "00".to_string()));
        assert_eq!(at(9), ("09AM".to_string(), "09".to_string()));
        assert_eq!(at(12), ("12PM".to_string(), "12".to_string()));
        assert_eq!(at(23), ("11PM".to_string(), "23".to_string()));
    }

    #[test]
    fn local_time_is_the_default_and_other_zones_fall_back_to_it() {
        let mut parser = NameParser::new(NameParserType::Default);
        parser.now = Some(monday_afternoon());
        let local = monday_afternoon().with_timezone(&Local);
        let expected = format!("{:02}-{:02}", local.hour(), local.minute());
        assert_eq!(parser.parse("%h-%mi").unwrap(), expected);
        parser.custom_time_zone = Some(CustomTimeZone::from_id("Central European Standard Time"));
        assert_eq!(parser.parse("%h-%mi").unwrap(), expected);
        assert_eq!(CustomTimeZone::from_id("UTC"), CustomTimeZone::Utc);
        assert_eq!(CustomTimeZone::from_id("Etc/UTC"), CustomTimeZone::Utc);
        // %unix is UTC whatever the zone.
        assert_eq!(parser.parse("%unix").unwrap(), "1792418709");
    }

    #[test]
    fn empty_patterns_and_unknown_codes() {
        assert_eq!(parse_at(NameParserType::FileName, ""), "");
        assert_eq!(
            parse_at(NameParserType::Default, "plain %q %"),
            "plain %q %"
        );
        assert_eq!(parse(NameParserType::Default, "").unwrap(), "");
    }

    #[test]
    fn window_title_and_process_name() {
        let mut p = parser(NameParserType::FileName);
        // Windows' file names cannot hold `|` or `:`; macOS' can.
        #[cfg(not(windows))]
        assert_eq!(p.parse("%t|%pn").unwrap(), "%t|%pn");
        #[cfg(windows)]
        assert_eq!(p.parse("%t|%pn").unwrap(), "%t%pn");
        p.window_text = Some("  My / Window: x  ".to_string());
        p.process_name = Some(" Fire fox ".to_string());
        #[cfg(not(windows))]
        assert_eq!(p.parse("%pn_%t").unwrap(), "Fire_fox_My__Window:_x");
        #[cfg(windows)]
        assert_eq!(p.parse("%pn_%t").unwrap(), "Fire_fox_My__Window_x");
        p.max_title_length = 4;
        assert_eq!(p.parse("%t").unwrap(), "My__");
        // Codes inside the title expand: it is replaced first.
        p.max_title_length = 0;
        p.window_text = Some("Report %y".to_string());
        assert_eq!(p.parse("%t").unwrap(), "Report_2026");
        // Paths keep their separators but not ones from the title.
        let mut p = parser(NameParserType::FilePath);
        p.window_text = Some("a/b".to_string());
        assert_eq!(p.parse("%y/%t").unwrap(), "2026/ab");
        // Text keeps the title as it is, apart from the spaces.
        let mut p = parser(NameParserType::Text);
        p.window_text = Some(" a/b c ".to_string());
        assert_eq!(p.parse("[%t]").unwrap(), "[a/b_c]");
    }

    #[test]
    fn image_size() {
        let mut p = parser(NameParserType::Default);
        assert_eq!(p.parse("%widthx%height").unwrap(), "x");
        p.image_width = 1920;
        p.image_height = 1080;
        assert_eq!(p.parse("%widthx%height").unwrap(), "1920x1080");
        // %width goes first, so %w does not eat its start.
        set_thread_culture(Some(Culture::english()));
        p.image_width = -5;
        assert_eq!(p.parse("%width|%w").unwrap(), "|Monday");
        set_thread_culture(None);
    }

    #[test]
    fn counters() {
        let mut p = parser(NameParserType::Default);
        p.auto_increment_number = 9;
        assert_eq!(p.parse("%i").unwrap(), "10");
        assert_eq!(p.auto_increment_number, 10);
        // A pattern without %i leaves the counter alone.
        assert_eq!(p.parse("%y").unwrap(), "2026");
        assert_eq!(p.auto_increment_number, 10);

        let mut at = |number: i32, pattern: &str| {
            p.auto_increment_number = number - 1;
            p.parse(pattern).unwrap()
        };
        assert_eq!(at(5, "%i{3}"), "005");
        assert_eq!(at(255, "%ix %iX %ix{4} %iX{4}"), "ff FF 00ff 00FF");
        assert_eq!(at(61, "%iAa %iaA"), "z Z");
        assert_eq!(at(61, "%iAa{3} %iaA{3}"), "00z 00Z");
        assert_eq!(at(35, "%ia %iA %ia{3} %iA{3}"), "z Z 00z 00Z");
        assert_eq!(at(255, "%ib{16,4} %iB{16,4}"), "00ff 00FF");
        assert_eq!(at(5, "%ib{2}"), "101");
        assert_eq!(at(7, "%i %i"), "7 7");
        assert_eq!(at(5, "%i{abc}"), "5");
        // The counter in every form in one pattern goes up once.
        assert_eq!(at(10, "%i-%ia-%iA-%ix-%iX-%iAa"), "10-a-A-a-A-A");
    }

    #[test]
    fn broken_base_codes_are_left_to_plain_i() {
        let mut p = parser(NameParserType::Default);
        p.auto_increment_number = 4;
        // Radix 1 throws inside upstream's try block: both base loops stop there and
        // the "%i" of what is left becomes the number.
        assert_eq!(p.parse("%ib{1,2}_%iB{16,2}").unwrap(), "5b{1,2}_5B{16,2}");
        p.auto_increment_number = 4;
        assert_eq!(p.parse("%ib{16,-1}").unwrap(), "5b{16,-1}");
        p.auto_increment_number = 4;
        assert_eq!(p.parse("%ib{}").unwrap(), "5b{}");
    }

    #[test]
    fn counters_fail_where_dotnet_throws() {
        let mut p = parser(NameParserType::Default);
        assert!(matches!(
            p.parse("%ia{-1}"),
            Err(NameParserError::InvalidNumberFormat(_))
        ));
        assert!(matches!(
            p.parse("%i{1000000000}"),
            Err(NameParserError::InvalidNumberFormat(_))
        ));
        assert_eq!(
            p.parse("%i{2000000}"),
            Err(NameParserError::TooLong(2_000_000))
        );
        // int.MaxValue + 1 wraps; the magnitude of int.MinValue does not fit, and
        // upstream computes the plain %iAa replacement for any counter pattern.
        p.auto_increment_number = i32::MAX;
        assert!(matches!(
            p.parse("%i"),
            Err(NameParserError::InvalidNumberFormat(_))
        ));
        assert_eq!(p.auto_increment_number, i32::MIN);
    }

    #[test]
    fn negative_counters_print_like_dotnet() {
        let mut p = parser(NameParserType::Default);
        p.auto_increment_number = -5;
        assert_eq!(
            p.parse("%i %i{3} %ix %iX{10}").unwrap(),
            "-4 -004 fffffffc 00FFFFFFFC"
        );
        p.auto_increment_number = -5;
        // ToBase writes the magnitude.
        assert_eq!(p.parse("%iAa").unwrap(), "4");
        p.auto_increment_number = -1;
        // Zero: ToBase gives nothing, the padding still pads.
        assert_eq!(p.parse("[%iAa][%iAa{3}][%i]").unwrap(), "[][000][0]");
    }

    #[test]
    fn negative_precisions_are_custom_formats() {
        assert_eq!(custom_format(5, "d-1"), "d-1");
        assert_eq!(custom_format(5, "x-10"), "x-15");
        assert_eq!(custom_format(123, "x-10"), "x-1123");
        assert_eq!(custom_format(5, "d-100"), "d-105");
        assert_eq!(custom_format(123, "d-1001"), "d-11231");
        assert_eq!(custom_format(0, "d-10"), "d-10");
        assert_eq!(custom_format(-5, "X-10"), "-X-15");
        let mut p = parser(NameParserType::Default);
        p.auto_increment_number = 4;
        assert_eq!(p.parse("%i{-1}").unwrap(), "d-1");
        p.auto_increment_number = 4;
        assert_eq!(p.parse("%ix{-10}").unwrap(), "x-15");
    }

    #[test]
    fn random_codes_by_length_and_alphabet() {
        let p = |pattern| parse_at(NameParserType::Default, pattern);
        let check = |text: String, length: usize, alphabet: &str| {
            assert_eq!(text.chars().count(), length, "{text}");
            assert!(text.chars().all(|c| alphabet.contains(c)), "{text}");
        };
        check(p("%rn"), 1, NUMBERS);
        check(p("%rn{12}"), 12, NUMBERS);
        check(p("%ra{20}"), 20, ALPHANUMERIC);
        check(p("%rna{30}"), 30, BASE56);
        check(p("%rx{16}"), 16, HEXADECIMAL_LOWER);
        check(p("%rX{16}"), 16, HEXADECIMAL_UPPER);
        check(p("%rn%rn%rn"), 3, NUMBERS);
        check(p("%rna%ra"), 2, ALPHANUMERIC);
        assert_eq!(p("[%rn{0}][%ra{-3}][%rx{x}]"), "[][][]");
        // Each occurrence gets its own draw.
        let many = p(&"%ra{8}-".repeat(20));
        let parts: std::collections::HashSet<&str> = many.split('-').collect();
        assert!(parts.len() > 2);
        assert_eq!(
            parser(NameParserType::Default).parse("%ra{2000000}"),
            Err(NameParserError::TooLong(2_000_000))
        );
    }

    #[test]
    fn guids_words_and_emoji() {
        let p = |pattern| parse_at(NameParserType::Default, pattern);
        let guid = p("%guid");
        assert_eq!(guid.len(), 36);
        let groups: Vec<&str> = guid.split('-').collect();
        assert_eq!(
            groups.iter().map(|g| g.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12]
        );
        assert!(groups[2].starts_with('4'));
        assert!(matches!(
            groups[3].chars().next(),
            Some('8' | '9' | 'a' | 'b')
        ));
        assert!(
            guid.chars()
                .all(|c| c == '-' || HEXADECIMAL_LOWER.contains(c))
        );
        let upper = p("%GUID");
        assert!(
            upper
                .chars()
                .all(|c| c == '-' || HEXADECIMAL_UPPER.contains(c))
        );
        assert_eq!(p("%Guid"), "%Guid");
        assert_ne!(p("%guid"), p("%guid"));

        let adjective = p("%radjective");
        assert!(adjective.chars().next().unwrap().is_ascii_uppercase());
        assert!(!adjective.contains('%'));
        let animal = p("%ranimal");
        assert!(animal.chars().next().unwrap().is_ascii_uppercase());

        let emoji = p("%remoji");
        assert!(super::super::helpers::EMOJIS.contains(&emoji.as_str()));
        let three = p("%remoji{3}");
        assert!(!three.is_empty() && !three.contains('%'));
        assert_eq!(p("%remoji{0}"), "");
    }

    #[test]
    fn random_lines_from_files() {
        set_thread_language(Some(Language::English));
        let dir = tempfile::tempdir().unwrap();
        let words = dir.path().join("words.txt");
        std::fs::write(&words, "alpha\r\nbeta\n\ngamma\n").unwrap();
        let words = words.to_string_lossy().into_owned();
        let mut p = parser(NameParserType::Default);
        for _ in 0..20 {
            let line = p.parse(&format!("%rf{{{words}}}")).unwrap();
            assert!(
                ["alpha", "beta", "", "gamma"].contains(&line.as_str()),
                "{line}"
            );
        }
        // The argument stops at the first comma.
        let line = p.parse(&format!("%rf{{{words},ignored}}")).unwrap();
        assert!(["alpha", "beta", "", "gamma"].contains(&line.as_str()));

        // Not a text file by its extension.
        let message = t(strings::NameParser_Valid_text_file_required).to_string();
        assert_eq!(
            p.parse("%rf{/tmp/picture.png}"),
            Err(NameParserError::RandomLineFile(message.clone()))
        );
        let missing = dir
            .path()
            .join("missing.txt")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            p.parse(&format!("%rf{{{missing}}}")),
            Err(NameParserError::RandomLineFile(format!(
                "Could not find file '{missing}'."
            )))
        );
        let lost = dir
            .path()
            .join("no")
            .join("such.txt")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            p.parse(&format!("%rf{{{lost}}}")),
            Err(NameParserError::RandomLineFile(format!(
                "Could not find a part of the path '{lost}'."
            )))
        );

        // The settings preview shows the message in place of the code.
        p.is_preview_mode = true;
        assert_eq!(p.parse("x %rf{a.png} y").unwrap(), format!("x {message} y"));
        set_thread_language(Some(Language::Polish));
        assert_eq!(
            p.parse("%rf{a.png}").unwrap(),
            "Wymagana jest prawidłowa ścieżka do pliku tekstowego."
        );
        set_thread_language(None);

        // Untrusted patterns may not read files at all.
        p.allow_file_read = false;
        assert_eq!(p.parse(&format!("<%rf{{{words}}}>")).unwrap(), "<>");
        // Without braces the code is left alone.
        assert_eq!(p.parse("%rf").unwrap(), "%rf");
    }

    #[test]
    fn computer_codes() {
        let p = |pattern| parse_at(NameParserType::Default, pattern);
        assert_eq!(p("%un"), user_name());
        assert_eq!(p("%uln"), user_domain_name());
        assert_eq!(p("%cn"), machine_name());
        // %un must not eat the start of %unix.
        assert_eq!(p("%unix"), "1792418709");
        #[cfg(not(windows))]
        {
            assert_eq!(user_domain_name(), machine_name());
            set_machine_name("Marcins-MacBook-Pro.local");
            assert_eq!(machine_name(), "Marcins-MacBook-Pro");
            assert_eq!(p("%cn|%uln"), "Marcins-MacBook-Pro|Marcins-MacBook-Pro");
        }
        #[cfg(windows)]
        {
            // The system's names, as .NET gives them on Windows.
            assert_eq!(
                system_machine_name(),
                std::env::var("COMPUTERNAME").unwrap()
            );
            assert_eq!(
                user_name().to_lowercase(),
                std::env::var("USERNAME").unwrap().to_lowercase()
            );
            assert_eq!(
                user_domain_name().to_lowercase(),
                std::env::var("USERDOMAIN").unwrap().to_lowercase()
            );
        }
    }

    #[test]
    fn new_lines_only_in_text() {
        #[cfg(not(windows))]
        assert_eq!(parse_at(NameParserType::Text, "a%nb"), "a\nb");
        #[cfg(windows)]
        assert_eq!(parse_at(NameParserType::Text, "a%nb"), "a\r\nb");
        assert_eq!(parse_at(NameParserType::Default, "a%nb"), "a%nb");
        assert_eq!(parse_at(NameParserType::FileName, "a%nb"), "a%nb");
    }

    #[test]
    fn results_are_cleaned_for_their_use() {
        assert_eq!(
            parse_at(NameParserType::FileName, "  %y/%mo\0x  "),
            "202610x"
        );
        assert_eq!(parse_at(NameParserType::FilePath, " %y/%mo\0 "), "2026/10");
        assert_eq!(parse_at(NameParserType::Default, " a/b "), " a/b ");
        assert_eq!(
            parse_at(NameParserType::Url, "https://example.com/%y/a b?q=a b"),
            "https://example.com/2026/a%20b?q=a b"
        );
        assert_eq!(
            parse_at(NameParserType::Url, "zdjęcie %y"),
            "zdj%c4%99cie%202026"
        );
    }

    #[test]
    fn names_are_cut_to_the_maximum_length() {
        let mut p = parser(NameParserType::FileName);
        p.max_name_length = 6;
        assert_eq!(p.parse("%y-%mo-%d").unwrap(), "2026-1");
        p.max_name_length = 0;
        assert_eq!(p.parse("%y-%mo-%d").unwrap(), "2026-10-19");
    }

    #[test]
    fn every_menu_code_is_understood() {
        assert_eq!(
            CODE_MENU_ENTRIES
                .iter()
                .map(|entry| entry.value)
                .collect::<Vec<_>>()
                .join(" "),
            "t pn y yy mo mon mon2 w w2 wy d h mi s ms pm unix i ia iAa ib ix rn ra rna rx \
             guid radjective ranimal remoji rf width height un uln cn n"
        );
        assert_eq!(codes::N.category, None);
        assert_eq!(codes::Y.to_prefix_string(), "%y");
        assert_eq!(codes::Y.with_prefix("$"), "$y");
        let dir = tempfile::tempdir().unwrap();
        let words = dir.path().join("words.txt");
        std::fs::write(&words, "word").unwrap();
        for entry in CODE_MENU_ENTRIES {
            let code = entry.to_prefix_string();
            let pattern = match entry.value {
                "rf" => format!("{code}{{{}}}", words.to_string_lossy()),
                "ib" => format!("{code}{{16,2}}"),
                _ => code.clone(),
            };
            let mut p = parser(NameParserType::Text);
            p.window_text = Some("Title".to_string());
            p.process_name = Some("app".to_string());
            p.image_width = 1;
            p.image_height = 1;
            let out = p.parse(&format!("[{pattern}]")).unwrap();
            assert!(!out.contains(&code), "{code} gave {out}");
            assert_ne!(out, format!("[{pattern}]"), "{code}");
            // Every code has its menu text in both languages.
            assert!(!crate::l10n::text_in(entry.description, Language::English).is_empty());
            assert!(!crate::l10n::text_in(entry.description, Language::Polish).is_empty());
        }
    }

    #[test]
    fn errors_read_like_their_dotnet_messages() {
        assert_eq!(
            NameParserError::RandomLineFile("x".to_string()).to_string(),
            "x"
        );
        assert_eq!(
            pad_zeroes("1", -1),
            Err(NameParserError::InvalidNumberFormat(
                "totalWidth ('-1') must be a non-negative value. (Parameter 'totalWidth')\nActual value was -1."
                    .to_string()
            ))
        );
        assert!(NameParserError::TooLong(5).to_string().contains('5'));
        assert!(
            NameParserError::Endless("%rf{a}".to_string())
                .to_string()
                .contains("%rf{a}")
        );
    }
}
