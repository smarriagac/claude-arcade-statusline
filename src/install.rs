//! `install` / `uninstall`: merge the `statusLine` entry into
//! `~/.claude/settings.json`, preserving every other key and its order.

use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

const KEY: &str = "statusLine";

/// The settings file as read: raw text (for backups) and the parsed object.
type RawSettings = (String, Map<String, Value>);

/// Install backups: `settings.json.arcade-backup-<UTC timestamp>`.
pub const BACKUP_PREFIX: &str = "settings.json.arcade-backup-";
/// Safety copies taken by `uninstall`; never used as a restore source.
pub const UNINSTALL_PREFIX: &str = "settings.json.arcade-uninstall-";

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub settings: PathBuf,
    pub backup: Option<PathBuf>,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub written: bool,
    /// One-line summary of what happened.
    pub summary: String,
}

pub fn settings_path(home: &Path) -> PathBuf {
    home.join(".claude").join("settings.json")
}

/// The `statusLine` value written by `install`.
pub fn status_line_value(command: &str) -> Value {
    serde_json::json!({"type": "command", "command": command, "refreshInterval": 1})
}

/// Quotes a binary path for the shell Claude Code runs the command in.
pub fn shell_command(exe: &Path) -> String {
    let path = exe.to_string_lossy();
    let plain = |c: char| c.is_ascii_alphanumeric() || "/._-+,:@%".contains(c);
    if cfg!(windows) {
        if path.contains(' ') {
            format!("\"{path}\"")
        } else {
            path.into_owned()
        }
    } else if path.chars().all(plain) {
        path.into_owned()
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

/// True when a `statusLine` value runs arcade-statusline.
fn is_ours(value: &Value) -> bool {
    value
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| c.contains("arcade-statusline"))
}

/// Raw text plus parsed object, or `None` when the file does not exist.
/// Invalid JSON or a non-object root is an error, so nothing gets clobbered.
fn read_settings(path: &Path) -> Result<Option<RawSettings>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    if raw.trim().is_empty() {
        return Ok(Some((raw, Map::new())));
    }
    match serde_json::from_str(&raw) {
        Ok(Value::Object(map)) => Ok(Some((raw, map))),
        Ok(_) => Err(format!(
            "{} is not a JSON object; left unchanged",
            path.display()
        )),
        Err(e) => Err(format!(
            "{} is not valid JSON ({e}); left unchanged",
            path.display()
        )),
    }
}

/// Writes through a temp file and a rename, following a symlinked
/// settings.json (dotfile managers) and keeping its permissions.
fn write_settings(path: &Path, map: Map<String, Value>) -> Result<(), String> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let mut text = serde_json::to_string_pretty(&Value::Object(map))
        .map_err(|e| format!("cannot serialize settings: {e}"))?;
    text.push('\n');
    let tmp = target.with_extension("json.arcade-tmp");
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", target.display());
    fs::write(&tmp, text).map_err(fail)?;
    if let Ok(meta) = fs::metadata(&target) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }
    fs::rename(&tmp, &target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        fail(e)
    })
}

/// Howard Hinnant's days-from-epoch to (year, month, day), proleptic Gregorian.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// UTC `YYYYMMDD-HHMMSS-mmm`: fixed width, so names sort chronologically.
fn timestamp(at: SystemTime) -> String {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let (y, mo, d) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!(
        "{y:04}{mo:02}{d:02}-{:02}{:02}{:02}-{:03}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        since.subsec_millis()
    )
}

/// Writes `raw` to a new `<dir>/<prefix><timestamp>`, moving the timestamp
/// forward a millisecond at a time if the name is taken.
fn write_backup(dir: &Path, prefix: &str, raw: &str, now: SystemTime) -> Result<PathBuf, String> {
    for step in 0..10_000 {
        let path = dir.join(format!(
            "{prefix}{}",
            timestamp(now + Duration::from_millis(step))
        ));
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path);
        match file {
            Ok(mut file) => {
                return file
                    .write_all(raw.as_bytes())
                    .map(|()| path.clone())
                    .map_err(|e| format!("cannot write backup {}: {e}", path.display()));
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot write backup {}: {e}", path.display())),
        }
    }
    Err(format!(
        "cannot find a free backup name in {}",
        dir.display()
    ))
}

/// The `statusLine` from before arcade-statusline: the newest install backup
/// whose entry is not ours (`None` inside means there was no entry).
fn previous_status_line(dir: &Path) -> Option<(Option<Value>, PathBuf)> {
    let mut backups: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(BACKUP_PREFIX))
        })
        .collect();
    backups.sort_unstable_by(|a, b| b.cmp(a));
    backups.into_iter().find_map(|path| {
        let text = fs::read_to_string(&path).ok()?;
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) else {
            return None;
        };
        match map.get(KEY) {
            Some(value) if is_ours(value) => None,
            value => Some((value.cloned(), path)),
        }
    })
}

pub fn install(
    home: &Path,
    command: &str,
    dry_run: bool,
    now: SystemTime,
) -> Result<Outcome, String> {
    let settings = settings_path(home);
    let original = read_settings(&settings)?;
    let mut map = original.as_ref().map(|o| o.1.clone()).unwrap_or_default();
    let before = map.get(KEY).cloned();
    let after = status_line_value(command);
    let mut outcome = Outcome {
        settings,
        backup: None,
        before,
        after: Some(after.clone()),
        written: false,
        summary: String::new(),
    };
    if outcome.before.as_ref() == Some(&after) {
        outcome.summary = "already installed; nothing changed".into();
        return Ok(outcome);
    }
    // Keeps the key's position when it already exists.
    map.insert(KEY.into(), after);
    if dry_run {
        outcome.summary = "dry run: nothing written".into();
        return Ok(outcome);
    }
    if let (Some((raw, _)), Some(dir)) = (&original, outcome.settings.parent()) {
        outcome.backup = Some(write_backup(dir, BACKUP_PREFIX, raw, now)?);
    }
    write_settings(&outcome.settings, map)?;
    outcome.written = true;
    outcome.summary = "installed: Claude Code's status line now runs arcade-statusline".into();
    Ok(outcome)
}

pub fn uninstall(home: &Path, now: SystemTime) -> Result<Outcome, String> {
    let settings = settings_path(home);
    let mut outcome = Outcome {
        settings,
        backup: None,
        before: None,
        after: None,
        written: false,
        summary: String::new(),
    };
    let Some((raw, mut map)) = read_settings(&outcome.settings)? else {
        outcome.summary = "no settings.json; nothing to uninstall".into();
        return Ok(outcome);
    };
    outcome.before = map.get(KEY).cloned();
    outcome.after = outcome.before.clone();
    if !outcome.before.as_ref().is_some_and(is_ours) {
        outcome.summary = "statusLine is not arcade-statusline; left unchanged".into();
        return Ok(outcome);
    }
    let dir = outcome.settings.parent().unwrap_or(home).to_path_buf();
    let previous = previous_status_line(&dir);
    outcome.after = previous.as_ref().and_then(|p| p.0.clone());
    outcome.summary = match (&previous, &outcome.after) {
        (Some((_, from)), Some(_)) => format!(
            "restored the previous statusLine from {}",
            from.file_name().unwrap_or_default().to_string_lossy()
        ),
        _ => "removed statusLine (there was none before install)".into(),
    };
    match &outcome.after {
        Some(value) => {
            map.insert(KEY.into(), value.clone());
        }
        None => {
            map.shift_remove(KEY);
        }
    }
    outcome.backup = Some(write_backup(&dir, UNINSTALL_PREFIX, &raw, now)?);
    write_settings(&outcome.settings, map)?;
    outcome.written = true;
    Ok(outcome)
}

/// Human-readable report of an outcome, including the statusLine diff.
pub fn describe(outcome: &Outcome, dry_run: bool) -> String {
    let mut out = format!("settings: {}\n", outcome.settings.display());
    if outcome.before != outcome.after {
        if let Some(before) = &outcome.before {
            out.push_str(&format!("- \"{KEY}\": {before}\n"));
        }
        if let Some(after) = &outcome.after {
            out.push_str(&format!("+ \"{KEY}\": {after}\n"));
        }
    }
    if let Some(backup) = &outcome.backup {
        out.push_str(&format!("backup:   {}\n", backup.display()));
    }
    out.push_str(&outcome.summary);
    out.push('\n');
    if dry_run && !outcome.written && outcome.before != outcome.after {
        out.push_str("run without --dry-run to apply\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, UNIX_EPOCH};

    fn temp_home(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "arcade-install-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(secs * 1000 + 123)
    }

    fn backups(home: &Path, prefix: &str) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = fs::read_dir(home.join(".claude"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(prefix))
            .collect();
        found.sort();
        found
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    const CMD: &str = "/opt/bin/arcade-statusline";

    #[test]
    fn install_creates_settings_when_missing() {
        let home = temp_home("create");
        let out = install(&home, CMD, false, at(1_800_000_000)).unwrap();
        assert!(out.written);
        assert!(out.backup.is_none());
        let json = read_json(&settings_path(&home));
        assert_eq!(
            json,
            serde_json::json!({"statusLine": status_line_value(CMD)})
        );
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn install_preserves_other_keys_and_writes_backup_first() {
        let home = temp_home("merge");
        let path = settings_path(&home);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = r#"{
  "zeta": 1,
  "statusLine": {"type": "command", "command": "bash ~/old.sh"},
  "alpha": {"nested": [1, 2, 3]},
  "model": "opus"
}
"#;
        fs::write(&path, original).unwrap();

        let out = install(&home, CMD, false, at(1_800_000_000)).unwrap();
        assert!(out.written);
        let backup = out.backup.clone().unwrap();
        assert_eq!(
            backup.file_name().unwrap().to_string_lossy(),
            "settings.json.arcade-backup-20270115-080000-123"
        );
        assert_eq!(fs::read_to_string(&backup).unwrap(), original);

        let json = read_json(&path);
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["zeta", "statusLine", "alpha", "model"]);
        assert_eq!(json["alpha"], serde_json::json!({"nested": [1, 2, 3]}));
        assert_eq!(json["model"], "opus");
        assert_eq!(json["statusLine"], status_line_value(CMD));
        assert_eq!(
            out.before,
            Some(serde_json::json!({"type": "command", "command": "bash ~/old.sh"}))
        );

        let report = describe(&out, false);
        assert!(
            report
                .contains("- \"statusLine\": {\"type\":\"command\",\"command\":\"bash ~/old.sh\"}")
        );
        assert!(report.contains(&format!("+ \"statusLine\": {}", status_line_value(CMD))));
        assert!(report.contains("backup"));
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn dry_run_writes_nothing() {
        let home = temp_home("dry");
        let path = settings_path(&home);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{\"a\": true}\n").unwrap();

        let out = install(&home, CMD, true, at(1_800_000_000)).unwrap();
        assert!(!out.written);
        assert!(out.backup.is_none());
        assert_eq!(out.after.as_ref().unwrap(), &status_line_value(CMD));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\": true}\n");
        assert!(backups(&home, "settings.json.arcade").is_empty());
        assert!(describe(&out, true).contains("dry run"));

        let fresh = temp_home("dry-missing");
        install(&fresh, CMD, true, at(1_800_000_000)).unwrap();
        assert!(!settings_path(&fresh).exists());
        fs::remove_dir_all(&home).unwrap();
        fs::remove_dir_all(&fresh).unwrap();
    }

    #[test]
    fn reinstall_with_same_command_changes_nothing() {
        let home = temp_home("again");
        install(&home, CMD, false, at(1_800_000_000)).unwrap();
        let out = install(&home, CMD, false, at(1_800_000_001)).unwrap();
        assert!(!out.written);
        assert!(backups(&home, BACKUP_PREFIX).is_empty());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn invalid_settings_are_left_untouched() {
        let home = temp_home("invalid");
        let path = settings_path(&home);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ not json").unwrap();
        assert!(install(&home, CMD, false, at(1_800_000_000)).is_err());
        fs::write(&path, "[1, 2]").unwrap();
        assert!(install(&home, CMD, false, at(1_800_000_000)).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "[1, 2]");
        assert!(backups(&home, "settings.json.arcade").is_empty());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn uninstall_restores_previous_status_line_from_latest_backup() {
        let home = temp_home("restore");
        let path = settings_path(&home);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"statusLine": {"type": "command", "command": "old"}, "k": 1}"#,
        )
        .unwrap();
        install(&home, CMD, false, at(1_800_000_000)).unwrap();
        // Reinstalling from another path backs up our own entry; uninstall
        // must skip that backup and restore the user's original command.
        install(&home, "/other/arcade-statusline", false, at(1_800_000_100)).unwrap();
        assert_eq!(backups(&home, BACKUP_PREFIX).len(), 2);

        let out = uninstall(&home, at(1_800_000_200)).unwrap();
        assert!(out.written);
        let json = read_json(&path);
        assert_eq!(
            json["statusLine"],
            serde_json::json!({"type": "command", "command": "old"})
        );
        assert_eq!(json["k"], 1);
        assert_eq!(backups(&home, UNINSTALL_PREFIX).len(), 1);
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn uninstall_removes_key_when_there_was_none() {
        let home = temp_home("remove");
        let path = settings_path(&home);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r#"{"k": 1}"#).unwrap();
        install(&home, CMD, false, at(1_800_000_000)).unwrap();
        let out = uninstall(&home, at(1_800_000_100)).unwrap();
        assert!(out.written);
        assert_eq!(read_json(&path), serde_json::json!({"k": 1}));

        // Fresh install without any prior settings file: no backup, key removed.
        let fresh = temp_home("remove-fresh");
        install(&fresh, CMD, false, at(1_800_000_000)).unwrap();
        uninstall(&fresh, at(1_800_000_100)).unwrap();
        assert_eq!(read_json(&settings_path(&fresh)), serde_json::json!({}));
        fs::remove_dir_all(&home).unwrap();
        fs::remove_dir_all(&fresh).unwrap();
    }

    #[test]
    fn uninstall_leaves_foreign_status_line_alone() {
        let home = temp_home("foreign");
        let path = settings_path(&home);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = r#"{"statusLine": {"type": "command", "command": "mine.sh"}}"#;
        fs::write(&path, text).unwrap();
        let out = uninstall(&home, at(1_800_000_000)).unwrap();
        assert!(!out.written);
        assert_eq!(fs::read_to_string(&path).unwrap(), text);

        let empty = temp_home("foreign-missing");
        assert!(!uninstall(&empty, at(1_800_000_000)).unwrap().written);
        fs::remove_dir_all(&home).unwrap();
        fs::remove_dir_all(&empty).unwrap();
    }

    #[test]
    fn shell_command_quotes_paths_with_spaces() {
        assert_eq!(
            shell_command(Path::new("/usr/local/bin/arcade-statusline")),
            "/usr/local/bin/arcade-statusline"
        );
        if !cfg!(windows) {
            assert_eq!(
                shell_command(Path::new("/Users/Jo Doe/bin/arcade-statusline")),
                "'/Users/Jo Doe/bin/arcade-statusline'"
            );
            assert_eq!(shell_command(Path::new("/it's/x")), "'/it'\\''s/x'");
        }
    }
}
