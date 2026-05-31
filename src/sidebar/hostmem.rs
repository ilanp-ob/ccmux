//! Host terminal app (iTerm2, Terminal, …) memory sampling for the ccmux footer.
//!
//! All process-spawning lives in the `sample_*` / `detect_*` wrappers at the
//! bottom; the logic above them is pure and unit-tested.

/// The terminal application hosting this ccmux session.
#[derive(Debug, Clone, PartialEq)]
pub struct HostApp {
    pub name: String,
    pub pid: u32,
}

/// Extract a display name from an executable path that lives inside a macOS
/// `.app` bundle. Returns `None` for non-app executables (shells, `login`, …).
pub fn friendly_name(exec_path: &str) -> Option<String> {
    if !exec_path.contains(".app/Contents/MacOS/") {
        return None;
    }
    let bundle = exec_path
        .split('/')
        .find(|seg| seg.ends_with(".app"))?
        .strip_suffix(".app")?;
    let friendly = match bundle {
        "iTerm" => "iTerm2",
        other => other,
    };
    Some(friendly.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_name_iterm() {
        assert_eq!(friendly_name("/Applications/iTerm.app/Contents/MacOS/iTerm2").as_deref(), Some("iTerm2"));
    }
    #[test]
    fn friendly_name_terminal() {
        assert_eq!(friendly_name("/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal").as_deref(), Some("Terminal"));
    }
    #[test]
    fn friendly_name_ghostty_keeps_bundle_name() {
        assert_eq!(friendly_name("/Applications/Ghostty.app/Contents/MacOS/ghostty").as_deref(), Some("Ghostty"));
    }
    #[test]
    fn friendly_name_non_app_is_none() {
        assert_eq!(friendly_name("/bin/zsh"), None);
        assert_eq!(friendly_name("login"), None);
    }
}
