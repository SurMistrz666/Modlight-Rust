//! The metadata viewer and strip metadata (`MetadataService`, `MetadataViewModel`) on
//! ExifTool, as upstream.
//!
//! On a Mac ExifTool is the Perl distribution the Components window installs (the
//! `exiftool` script and its `lib` folder in `Tools/ExifTool`, SHA-256 pinned), run by
//! `/usr/bin/perl`. The arguments are upstream's: `-G -t -m -q` to read, `-all=
//! -overwrite_original -q` to strip. Nothing passes through a shell.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use modlight_core::l10n::{Arg, format, t, tools as tl};

/// The Perl that runs ExifTool (macOS ships it).
pub const PERL: &str = "/usr/bin/perl";

/// One tag: `MetadataEntry`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub group: String,
    pub tag: String,
    pub value: String,
    pub url: Option<String>,
}

/// `MetadataGroup`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub entries: Vec<Entry>,
}

/// How ExifTool is started: the interpreter and the script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExifTool {
    pub program: PathBuf,
    pub script: Option<PathBuf>,
}

impl ExifTool {
    /// The installed component (`installed_path(profile, "exiftool")` is the script).
    pub fn installed(profile: &Path) -> Option<ExifTool> {
        let script = modlight_core::components::installed_path(profile, "exiftool")?;
        Some(ExifTool {
            program: PathBuf::from(PERL),
            script: Some(script),
        })
    }

    /// A program run directly (tests use a stand-in).
    pub fn direct(program: PathBuf) -> ExifTool {
        ExifTool {
            program,
            script: None,
        }
    }

    /// The argument vector for one file: the script, the file, then upstream's options.
    pub fn arguments(&self, file: &Path, options: &[&str]) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(script) = &self.script {
            args.push(script.to_string_lossy().into_owned());
        }
        args.push(absolute(file).to_string_lossy().into_owned());
        args.extend(options.iter().map(|s| s.to_string()));
        args
    }

    /// `RunExifToolAsync`: standard output, or the error text upstream shows.
    pub fn run(&self, file: &Path, options: &[&str]) -> Result<String, String> {
        if !self.program.is_file() || self.script.as_ref().is_some_and(|s| !s.is_file()) {
            return Err(t(tl::MetadataService_ExifTool_not_found).to_string());
        }
        if !file.is_file() {
            return Err(t(tl::MetadataService_Selected_file_not_found).to_string());
        }
        let output = Command::new(&self.program)
            .args(self.arguments(file, options))
            .stdin(Stdio::null())
            .output()
            .map_err(|_| t(tl::MetadataService_Failed_start_ExifTool).to_string())?;
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if error.is_empty() {
                format(
                    t(tl::MetadataService_Exited_with_code),
                    &[Arg::from(output.status.code().unwrap_or(-1) as i64)],
                )
            } else {
                error
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// `ReadMetadataAsync`.
    pub fn read(&self, file: &Path) -> Result<Vec<Entry>, String> {
        self.run(file, READ_OPTIONS).map(|text| parse(&text))
    }

    /// `StripMetadataAsync` / `StripFileMetadata`.
    pub fn strip(&self, file: &Path) -> Result<(), String> {
        self.run(file, STRIP_OPTIONS).map(|_| ())
    }
}

/// `-G -t -m -q`: group names, tab separated, minor warnings ignored, quiet.
pub const READ_OPTIONS: &[&str] = &["-G", "-t", "-m", "-q"];
/// `-all= -overwrite_original -q`.
pub const STRIP_OPTIONS: &[&str] = &["-all=", "-overwrite_original", "-q"];

/// A path ExifTool cannot read as an option.
fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

/// `ParseMetadata`: "Group<TAB>Tag<TAB>Value" lines, ExifTool's own group left out.
pub fn parse(text: &str) -> Vec<Entry> {
    let url = regex::Regex::new(r"(?i)https?://[^\s]+").expect("valid");
    text.split(['\r', '\n'])
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let group = parts.next()?.trim().to_string();
            let tag = parts.next()?.trim().to_string();
            if group.eq_ignore_ascii_case("ExifTool") {
                return None;
            }
            let value = parts.next().unwrap_or("").trim().to_string();
            let url = url.find(&value).map(|m| m.as_str().to_string());
            Some(Entry {
                group,
                tag,
                value,
                url,
            })
        })
        .collect()
}

/// `ApplyFilter`: entries whose group, tag or value contain the text, grouped by group
/// (case-insensitively, first spelling wins), in order of appearance.
pub fn groups(entries: &[Entry], filter: &str) -> Vec<Group> {
    let filter = filter.trim().to_lowercase();
    let mut out: Vec<Group> = Vec::new();
    for entry in entries {
        if !filter.is_empty()
            && !entry.group.to_lowercase().contains(&filter)
            && !entry.tag.to_lowercase().contains(&filter)
            && !entry.value.to_lowercase().contains(&filter)
        {
            continue;
        }
        match out
            .iter_mut()
            .find(|g| g.name.to_lowercase() == entry.group.to_lowercase())
        {
            Some(group) => group.entries.push(entry.clone()),
            None => out.push(Group {
                name: entry.group.clone(),
                entries: vec![entry.clone()],
            }),
        }
    }
    out
}

/// `BuildMetadataText`: "# Group" and "    Tag: Value" lines, a blank line between
/// (`AppendLine`: the platform's line break).
pub fn copy_text(entries: &[Entry]) -> String {
    let mut out = String::new();
    for group in groups(entries, "") {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("# {}\n", group.name));
        for entry in &group.entries {
            out.push_str(&format!("    {}: {}\n", entry.tag, entry.value));
        }
    }
    out.trim_end().to_string()
}

/// `MetadataCountText`.
pub fn count_text(count: usize) -> String {
    if count == 1 {
        t(tl::MetadataViewModel_One_tag).to_string()
    } else {
        format(t(tl::MetadataViewModel_Tag_count), &[Arg::from(count)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "ExifTool\tExifTool Version Number\t13.59\r\nFile\tFile Name\tsunset.jpg\r\nFile\tFile Size\t52 kB\r\nEXIF\tMake\tApple\r\nEXIF\tModel\tiPhone 15 Pro\r\nXMP\tCreator Tool\tSee https://example.com/tool now\r\nfile\tMIME Type\timage/jpeg\r\nComposite\tImage Size\t1600x900\r\n";

    #[test]
    fn parsing_and_grouping() {
        let entries = parse(SAMPLE);
        assert_eq!(entries.len(), 7);
        assert_eq!(entries[0].group, "File");
        assert_eq!(entries[4].url.as_deref(), Some("https://example.com/tool"));
        let groups = groups(&entries, "");
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["File", "EXIF", "XMP", "Composite"]);
        assert_eq!(groups[0].entries.len(), 3);
        let filtered = super::groups(&entries, "IPHONE");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].entries[0].tag, "Model");
    }

    #[test]
    fn copied_text() {
        let text = copy_text(&parse(SAMPLE));
        assert!(text.starts_with("# File\n    File Name: sunset.jpg\n    File Size: 52 kB\n    MIME Type: image/jpeg\n\n# EXIF\n"));
        assert!(text.ends_with("    Image Size: 1600x900"));
    }

    #[test]
    fn arguments() {
        let tool = ExifTool {
            program: PathBuf::from(PERL),
            script: Some(PathBuf::from("/T/ExifTool/exiftool")),
        };
        assert_eq!(
            tool.arguments(Path::new("/p/a b.jpg"), READ_OPTIONS),
            ["/T/ExifTool/exiftool", "/p/a b.jpg", "-G", "-t", "-m", "-q"]
        );
        assert_eq!(
            ExifTool::direct("/x".into()).arguments(Path::new("/p/-x.jpg"), STRIP_OPTIONS),
            ["/p/-x.jpg", "-all=", "-overwrite_original", "-q"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_stand_in_exiftool() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("args.txt");
        let script = dir.path().join("exiftool");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nif [ \"$2\" = \"-all=\" ]; then exit 0; fi\nif [ \"$1\" = \"{}/bad.jpg\" ]; then echo 'Error: File format error' >&2; exit 1; fi\nprintf 'File\\tFile Name\\tok.jpg\\n'\n",
                log.display(),
                dir.path().display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let tool = ExifTool::direct(script);
        let file = dir.path().join("ok.jpg");
        std::fs::write(&file, b"x").unwrap();
        let entries = tool.read(&file).unwrap();
        assert_eq!(entries[0].value, "ok.jpg");
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            format!("{}\n-G\n-t\n-m\n-q\n", file.display())
        );
        tool.strip(&file).unwrap();
        assert!(
            std::fs::read_to_string(&log)
                .unwrap()
                .contains("-overwrite_original")
        );
        let bad = dir.path().join("bad.jpg");
        std::fs::write(&bad, b"x").unwrap();
        assert_eq!(tool.read(&bad).unwrap_err(), "Error: File format error");
        assert!(tool.read(&dir.path().join("missing.jpg")).is_err());
    }
}
