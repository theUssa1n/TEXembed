use std::any::Any;
use std::backtrace::Backtrace;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static LOG_LOCK: Mutex<()> = Mutex::new(());

pub fn run_guarded<T>(operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match panic::catch_unwind(AssertUnwindSafe(operation)) {
        Ok(result) => result,
        Err(payload) => Err(format!(
            "Operation panicked: {}",
            panic_message(payload.as_ref())
        )),
    }
}

/// Canonicalizes `path` like [`fs::canonicalize`], but on Windows removes the
/// extended-length `\\?\` prefix whenever the plain form is equivalent, so
/// paths shown in the UI, logs and error messages look like `D:\Games\file`.
pub fn canonicalize_path(path: &Path) -> io::Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    #[cfg(windows)]
    {
        if let Some(simplified) = canonical.to_str().and_then(simplify_verbatim_path) {
            return Ok(PathBuf::from(simplified));
        }
    }
    Ok(canonical)
}

/// Turns `\\?\C:\dir\file` into `C:\dir\file` and `\\?\UNC\srv\share\x` into
/// `\\srv\share\x`. Returns `None` when the prefix must stay, which is the case
/// for non-drive verbatim paths, paths of 260 characters or more, reserved
/// device names, and components that only the verbatim form can address.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn simplify_verbatim_path(path: &str) -> Option<String> {
    const MAX_PATH: usize = 260;
    let rest = path.strip_prefix(r"\\?\")?;
    let (simplified, first_component) = if let Some(unc) = rest.strip_prefix(r"UNC\") {
        (format!(r"\\{unc}"), 0)
    } else {
        let bytes = rest.as_bytes();
        let is_drive_path = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\';
        if !is_drive_path {
            return None;
        }
        (rest.to_string(), 1)
    };
    if simplified.len() >= MAX_PATH {
        return None;
    }
    let body = simplified.trim_start_matches('\\');
    for component in body.split('\\').skip(first_component) {
        if component.is_empty()
            || component.ends_with(['.', ' '])
            || component.contains(['<', '>', ':', '"', '|', '?', '*', '/'])
            || is_reserved_device_name(component)
        {
            return None;
        }
    }
    Some(simplified)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn is_reserved_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or("").trim_end();
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0')
}

pub fn log_timestamp() -> String {
    timestamp_at(SystemTime::now())
}

pub fn install_panic_hook() {
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let thread = format!("{} ({:?})", thread.name().unwrap_or("unnamed"), thread.id());
        let location = info.location().map_or_else(
            || "unknown".to_string(),
            |location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            },
        );
        let report = panic_report(
            &log_timestamp(),
            &thread,
            &location,
            panic_message(info.payload()),
            &Backtrace::force_capture().to_string(),
        );

        if let Err(error) = append_report(&report) {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("TEXembed crash reporting failed")
                .set_description(format!(
                    "{}\n\nLocation: {location}\n\nCould not save crash.log: {error}",
                    panic_message(info.payload())
                ))
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
        }
        previous_hook(info);
    }));
}

pub fn log_error(message: &str) -> Result<PathBuf, String> {
    let thread = std::thread::current();
    append_report(&format!(
        "[{}] ERROR\nThread: {} ({:?})\n{message}\n\n",
        log_timestamp(),
        thread.name().unwrap_or("unnamed"),
        thread.id()
    ))
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    if let Some(message) = payload.downcast_ref::<String>() {
        message
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        message
    } else {
        "non-string panic payload"
    }
}

fn panic_report(
    timestamp: &str,
    thread: &str,
    location: &str,
    message: &str,
    backtrace: &str,
) -> String {
    format!(
        "=== TEXembed panic ===\nTimestamp: {timestamp}\nThread: {thread}\nLocation: {location}\nMessage: {message}\nBacktrace:\n{backtrace}\n\n"
    )
}

fn duration_nanos(duration: Duration) -> i128 {
    i128::from(duration.as_secs()) * 1_000_000_000 + i128::from(duration.subsec_nanos())
}

fn timestamp_at(time: SystemTime) -> String {
    let nanos = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration_nanos(duration),
        Err(error) => -duration_nanos(error.duration()),
    };
    let millis = nanos.div_euclid(1_000_000);
    let seconds = millis.div_euclid(1_000);
    let (year, month, day) = civil_date(seconds.div_euclid(86_400));
    let time_of_day = seconds.rem_euclid(86_400);
    let hours = time_of_day / 3_600;
    let minutes = time_of_day % 3_600 / 60;
    let seconds = time_of_day % 60;
    let millis = millis.rem_euclid(1_000);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z")
}

fn civil_date(days: i128) -> (i128, i128, i128) {
    let shifted_days = days + 719_468;
    let era = shifted_days.div_euclid(146_097);
    let day_of_era = shifted_days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = shifted_month + if shifted_month < 10 { 3 } else { -9 };
    (year + i128::from(month <= 2), month, day)
}

fn log_paths(roots: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in roots {
        if root.is_absolute() {
            let path = root.join("TEXembed").join("crash.log");
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

fn append_report(report: &str) -> Result<PathBuf, String> {
    let roots = ["LOCALAPPDATA", "APPDATA", "USERPROFILE", "HOME"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .chain(std::iter::once(std::env::temp_dir()));
    let paths = log_paths(roots);
    let _guard = acquire_log_lock(&LOG_LOCK, std::thread::panicking());
    append_report_to_paths(&paths, report)
}

fn acquire_log_lock(lock: &Mutex<()>, nonblocking: bool) -> Option<MutexGuard<'_, ()>> {
    if nonblocking {
        match lock.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(error)) => Some(error.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    } else {
        Some(lock.lock().unwrap_or_else(|error| error.into_inner()))
    }
}

fn append_report_to_paths(paths: &[PathBuf], report: &str) -> Result<PathBuf, String> {
    let mut errors = Vec::new();
    for path in paths {
        match append_file(path, report) {
            Ok(()) => return Ok(path.clone()),
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    if errors.is_empty() {
        Err("No absolute per-user log directory is available.".to_string())
    } else {
        Err(errors.join("; "))
    }
}

fn append_file(path: &Path, report: &str) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "The log path has no parent"))?;
    fs::create_dir_all(directory)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(report.as_bytes())?;
    file.sync_data()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "texembed-runtime-test-{}-{unique}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(canonicalize_path(&path).unwrap())
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn guarded_operations_preserve_values_and_errors() {
        assert_eq!(run_guarded(|| Ok(42)), Ok(42));
        assert_eq!(
            run_guarded::<()>(|| Err("expected error".to_string())),
            Err("expected error".to_string())
        );
    }

    #[test]
    fn guarded_operations_catch_panics_with_mutable_captures() {
        let mut progress = 0;
        let result = run_guarded::<()>(|| {
            progress = 7;
            panic!("decode failed");
        });
        assert_eq!(progress, 7);
        assert_eq!(result.unwrap_err(), "Operation panicked: decode failed");
    }

    #[test]
    fn guarded_operations_catch_owned_and_non_string_payloads() {
        let owned = run_guarded::<()>(|| panic::panic_any("owned failure".to_string()));
        let non_string = run_guarded::<()>(|| panic::panic_any(17_u32));
        assert!(owned.unwrap_err().contains("owned failure"));
        assert!(non_string.unwrap_err().contains("non-string panic payload"));
    }

    #[test]
    fn timestamps_handle_epoch_fractional_seconds_and_pre_epoch_values() {
        assert_eq!(timestamp_at(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            timestamp_at(UNIX_EPOCH + Duration::from_millis(3_723_456)),
            "1970-01-01T01:02:03.456Z"
        );
        assert_eq!(
            timestamp_at(UNIX_EPOCH - Duration::from_micros(1)),
            "1969-12-31T23:59:59.999Z"
        );
        assert_eq!(
            timestamp_at(UNIX_EPOCH - Duration::from_secs(86_400)),
            "1969-12-31T00:00:00.000Z"
        );
    }

    #[test]
    fn timestamps_handle_leap_years_and_century_boundaries() {
        assert_eq!(
            timestamp_at(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29T00:00:00.000Z"
        );
        assert_eq!(
            timestamp_at(UNIX_EPOCH + Duration::from_secs(4_107_542_400)),
            "2100-03-01T00:00:00.000Z"
        );
    }

    #[test]
    fn panic_reports_include_all_diagnostic_fields() {
        let report = panic_report("time", "worker (id)", "source.rs:12:3", "failure", "trace");
        for field in [
            "Timestamp: time",
            "Thread: worker (id)",
            "Location: source.rs:12:3",
            "Message: failure",
            "Backtrace:\ntrace",
        ] {
            assert!(report.contains(field));
        }
    }

    #[test]
    fn log_candidates_ignore_relative_roots_and_preserve_priority() {
        let directory = TestDirectory::new();
        let primary = directory.0.join("local");
        let fallback = directory.0.join("fallback");
        assert_eq!(
            log_paths([
                PathBuf::new(),
                PathBuf::from("relative"),
                primary.clone(),
                primary.clone(),
                fallback.clone(),
            ]),
            vec![
                primary.join("TEXembed").join("crash.log"),
                fallback.join("TEXembed").join("crash.log"),
            ]
        );
    }

    #[test]
    fn log_writes_fall_back_and_append_without_overwriting() {
        let directory = TestDirectory::new();
        let blocked = directory.0.join("blocked");
        fs::write(&blocked, b"not a directory").unwrap();
        let paths = log_paths([blocked, directory.0.clone()]);
        let written = append_report_to_paths(&paths, "first\n").unwrap();
        assert_eq!(written, paths[1]);
        assert_eq!(append_report_to_paths(&paths, "second\n").unwrap(), written);
        assert_eq!(fs::read_to_string(written).unwrap(), "first\nsecond\n");
    }

    #[test]
    fn log_writes_propagate_failures_when_no_candidate_works() {
        let directory = TestDirectory::new();
        let blocked = directory.0.join("blocked");
        fs::write(&blocked, b"not a directory").unwrap();
        let error = append_report_to_paths(&log_paths([blocked]), "report").unwrap_err();
        assert!(error.contains("crash.log"));
        assert!(append_report_to_paths(&[], "report").is_err());
    }

    #[test]
    fn panic_logging_does_not_wait_for_a_busy_log_lock() {
        let lock = Mutex::new(());
        let guard = lock.lock().unwrap();
        assert!(acquire_log_lock(&lock, true).is_none());
        drop(guard);
        assert!(acquire_log_lock(&lock, true).is_some());
    }

    #[test]
    fn poisoned_log_locks_remain_usable() {
        let lock = Mutex::new(());
        let result = run_guarded::<()>(|| {
            let _guard = lock.lock().unwrap();
            panic!("synthetic logging panic");
        });
        assert!(result.is_err());
        assert!(lock.is_poisoned());
        assert!(acquire_log_lock(&lock, true).is_some());
        assert!(acquire_log_lock(&lock, false).is_some());
    }

    #[test]
    fn simplify_verbatim_strips_drive_and_unc_prefixes() {
        assert_eq!(
            simplify_verbatim_path(r"\\?\D:\SSDC\Deferred\Skydome.textures").as_deref(),
            Some(r"D:\SSDC\Deferred\Skydome.textures")
        );
        assert_eq!(
            simplify_verbatim_path(r"\\?\UNC\server\share\dir\a.textures").as_deref(),
            Some(r"\\server\share\dir\a.textures")
        );
    }

    #[test]
    fn simplify_verbatim_keeps_prefix_when_plain_form_is_not_equivalent() {
        assert_eq!(simplify_verbatim_path(r"D:\already\plain"), None);
        assert_eq!(simplify_verbatim_path(r"\\?\Volume{1234}\file"), None);
        assert_eq!(simplify_verbatim_path(r"\\?\D:\dir\aux.textures"), None);
        assert_eq!(simplify_verbatim_path(r"\\?\D:\dir\NUL"), None);
        assert_eq!(simplify_verbatim_path(r"\\?\D:\dir\com1.txt"), None);
        assert_eq!(simplify_verbatim_path(r"\\?\D:\trailing.\file"), None);
        assert_eq!(simplify_verbatim_path(r"\\?\D:\trailing \file"), None);
        let long = format!(r"\\?\D:\{}", "a".repeat(300));
        assert_eq!(simplify_verbatim_path(&long), None);
    }

    #[test]
    fn simplify_verbatim_allows_names_that_only_resemble_reserved_ones() {
        assert_eq!(
            simplify_verbatim_path(r"\\?\D:\dir\console.textures").as_deref(),
            Some(r"D:\dir\console.textures")
        );
        assert_eq!(
            simplify_verbatim_path(r"\\?\D:\dir\com10.textures").as_deref(),
            Some(r"D:\dir\com10.textures")
        );
        assert_eq!(
            simplify_verbatim_path(r"\\?\D:\dir\com0.textures").as_deref(),
            Some(r"D:\dir\com0.textures")
        );
    }

    #[test]
    fn canonicalize_path_resolves_existing_files_without_verbatim_prefix() {
        let dir = TestDirectory::new();
        let file = dir.0.join("a.textures");
        fs::write(&file, b"x").unwrap();
        let resolved = canonicalize_path(&file).unwrap();
        assert!(resolved.is_absolute());
        assert!(!resolved.to_string_lossy().starts_with(r"\\?\"));
        assert!(canonicalize_path(&dir.0.join("missing")).is_err());
    }
}
