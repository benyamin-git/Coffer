use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EditorError {
    #[error("no editor found; install nano, or set $VISUAL or $EDITOR")]
    NoEditor,
    #[error("editor exited with status {0}")]
    Failed(i32),
    #[error("editor could not be started: {0}")]
    Spawn(String),
    #[error("failed to read back the edited note")]
    ReadBack(#[source] std::io::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

const FALLBACK_EDITORS: [&str; 3] = ["nano", "vim", "vi"];
const TEMP_PREFIX: &str = "coffer-edit-";

fn temp_dir() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(env::temp_dir)
}

/// Remove editor temp files whose owning process is gone, e.g. after an SSH
/// connection dropped mid-edit. Files belonging to live processes are kept,
/// so concurrently running coffer instances are never disturbed.
fn cleanup_stale_temp_files(dir: &std::path::Path) {
    // Liveness is checked through /proc; without it, delete nothing.
    if !std::path::Path::new("/proc").is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(rest) = name.strip_prefix(TEMP_PREFIX) else {
            continue;
        };
        let Some(pid) = rest.split('-').next().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        if !std::path::Path::new("/proc").join(pid.to_string()).exists() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn on_path(program: &str) -> bool {
    if program.contains('/') {
        return std::path::Path::new(program).is_file();
    }
    env::var_os("PATH")
        .map(|paths| {
            env::split_paths(&paths).any(|dir| {
                let candidate = dir.join(program);
                candidate.is_file()
            })
        })
        .unwrap_or(false)
}

/// Pick an editor: `$VISUAL`, then `$EDITOR`, then the first of nano, vim,
/// vi found on `PATH`. The bool says whether it came from the environment
/// rather than the fallback list.
fn choose_editor(
    visual: Option<String>,
    editor: Option<String>,
    exists: impl Fn(&str) -> bool,
) -> Option<(String, bool)> {
    let nonempty = |s: String| {
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    };
    if let Some(visual) = visual.and_then(nonempty) {
        return Some((visual, true));
    }
    if let Some(editor) = editor.and_then(nonempty) {
        return Some((editor, true));
    }
    FALLBACK_EDITORS
        .iter()
        .find(|program| exists(program))
        .map(|program| ((*program).to_string(), false))
}

/// Open the user's editor on the given text and return the edited result.
///
/// The text is written to an owner-only temporary file (preferring
/// `$XDG_RUNTIME_DIR`, which is a tmpfs on Linux), the editor runs
/// synchronously, and the file is deleted before this function returns.
pub fn edit(initial: &str) -> Result<String, EditorError> {
    let (editor, user_chosen) =
        choose_editor(env::var("VISUAL").ok(), env::var("EDITOR").ok(), on_path)
            .ok_or(EditorError::NoEditor)?;
    if !user_chosen && editor == "nano" {
        eprintln!("Opening nano - save with Ctrl+O then Enter, exit with Ctrl+X");
    }

    let dir = temp_dir();
    cleanup_stale_temp_files(&dir);

    let mut tmp = tempfile::Builder::new()
        .prefix(&format!("{TEMP_PREFIX}{}-", std::process::id()))
        .suffix(".txt")
        .tempfile_in(dir)?;
    tmp.write_all(initial.as_bytes())?;
    tmp.flush()?;
    let path = tmp.path().to_path_buf();

    // The path is passed as a positional argument, never interpolated into
    // the shell string, so odd characters in the path cannot be exploited.
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&path)
        .status()
        .map_err(|e| EditorError::Spawn(e.to_string()))?;
    if !status.success() {
        return Err(EditorError::Failed(status.code().unwrap_or(-1)));
    }

    // The editor may have replaced the file; re-tighten permissions before
    // reading, then remove it.
    if let Ok(meta) = std::fs::metadata(&path) {
        let mut perms = meta.permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o600);
        let _ = std::fs::set_permissions(&path, perms);
    }
    let content = std::fs::read_to_string(&path).map_err(EditorError::ReadBack)?;
    let _ = std::fs::remove_file(&path);

    Ok(content.trim_end_matches(['\n', '\r']).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(programs: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |program| programs.contains(&program)
    }

    #[test]
    fn visual_wins_over_editor_and_fallbacks() {
        let (editor, user_chosen) = choose_editor(
            Some("my-editor".to_string()),
            Some("other".to_string()),
            has(&["nano"]),
        )
        .expect("some editor");
        assert_eq!(editor, "my-editor");
        assert!(user_chosen);
    }

    #[test]
    fn editor_used_when_visual_is_missing() {
        let (editor, user_chosen) =
            choose_editor(None, Some("nano".to_string()), has(&["nano"])).expect("some editor");
        assert_eq!(editor, "nano");
        assert!(user_chosen);
    }

    #[test]
    fn blank_environment_values_are_ignored() {
        let (editor, user_chosen) =
            choose_editor(Some("  ".to_string()), Some("".to_string()), has(&["vim"]))
                .expect("some editor");
        assert_eq!(editor, "vim");
        assert!(!user_chosen);
    }

    #[test]
    fn fallback_order_is_nano_vim_vi() {
        let pick = |available: &'static [&'static str]| {
            choose_editor(None, None, has(available)).map(|(editor, _)| editor)
        };
        assert_eq!(pick(&["vi", "vim", "nano"]).as_deref(), Some("nano"));
        assert_eq!(pick(&["vi", "vim"]).as_deref(), Some("vim"));
        assert_eq!(pick(&["vi"]).as_deref(), Some("vi"));
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn fallback_editors_are_not_user_chosen() {
        let (editor, user_chosen) = choose_editor(None, None, has(&["nano"])).expect("some editor");
        assert_eq!(editor, "nano");
        assert!(!user_chosen);
    }

    #[test]
    fn stale_temp_files_are_removed_but_live_ones_are_kept() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let live = dir
            .path()
            .join(format!("{TEMP_PREFIX}{}-abc.txt", std::process::id()));
        let stale = dir.path().join(format!("{TEMP_PREFIX}4000000000-abc.txt"));
        let unrelated = dir.path().join("other-notes.txt");
        for file in [&live, &stale, &unrelated] {
            std::fs::write(file, b"x").expect("write");
        }

        cleanup_stale_temp_files(dir.path());

        assert!(live.exists(), "temp file of a live process was removed");
        assert!(!stale.exists(), "stale temp file was not removed");
        assert!(unrelated.exists(), "unrelated file was removed");
    }

    #[test]
    fn files_without_a_pid_are_left_alone() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let old_style = dir.path().join(format!("{TEMP_PREFIX}abcdef.txt"));
        std::fs::write(&old_style, b"x").expect("write");

        cleanup_stale_temp_files(dir.path());

        assert!(old_style.exists());
    }
}
