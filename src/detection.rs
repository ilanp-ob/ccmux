use crate::session::ClaudeCodeStatus;

/// Returns true when the pane content shows a Claude confirmation/permission
/// dialog that requires the user to respond before Claude can continue.
///
/// Dialog footers always render at the bottom of the terminal. We only check
/// the last few lines so code/text that *mentions* these phrases mid-output
/// (e.g. a code diff displaying the detection source itself) doesn't trigger.
fn is_waiting_for_input(content: &str) -> bool {
    // All checks are tail-only: Claude's dialog footers always appear at the
    // bottom of the terminal. Scanning the full capture causes false positives
    // when Claude displays code that contains these exact string literals
    // (e.g. a diff of detection.rs itself showing `[y/n]` or the footer phrases).
    let tail: Vec<&str> = content.lines().rev().take(6).collect();
    let tail_text = tail.join("\n");

    if tail_text.contains("[y/n]") || tail_text.contains("[Y/n]") {
        return true;
    }
    if tail_text.contains("Tab to amend · ctrl+e to explain") {
        return true;
    }
    if tail_text.contains("Enter to select · ↑/↓ to navigate") {
        return true;
    }
    false
}

/// Returns true when Claude's thinking/processing spinner is visible near the bottom of
/// the terminal. Covers the standard spinner animation (·, ✻, ✽, ✶, ✳, ✢ at line start)
/// as well as extended-thinking mode which renders "[ornament] Thinking…".
fn is_thinking(content: &str) -> bool {
    const SPINNERS: &[char] = &[
        '\u{00B7}', // · middle dot
        '\u{273B}', // ✻ teardrop-spoked asterisk
        '\u{273D}', // ✽ heavy teardrop-spoked asterisk
        '\u{2736}', // ✶ six-pointed black star
        '\u{2733}', // ✳ eight-spoked asterisk
        '\u{2722}', // ✢ four balloon-spoked asterisk
    ];
    content.lines().rev().take(20).any(|line| {
        let t = line.trim();
        // Extended thinking mode: "[ornament] Thinking…" (Unicode ellipsis U+2026)
        if t.ends_with("Thinking\u{2026}") && t.len() > "Thinking\u{2026}".len() {
            return true;
        }
        // Standard spinner: ornament char followed immediately by a space
        let mut chars = t.chars();
        matches!(chars.next(), Some(c) if SPINNERS.contains(&c)) && chars.next() == Some(' ')
    })
}

/// Status when content has CHANGED since last tick.
/// Safe assumption is Working; only override if a confirmation dialog or
/// extended-thinking spinner is visible.
pub fn detect_changed_status(content: &str) -> ClaudeCodeStatus {
    if is_waiting_for_input(content) {
        ClaudeCodeStatus::WaitingInput
    } else if is_thinking(content) {
        ClaudeCodeStatus::Thinking
    } else {
        ClaudeCodeStatus::Working
    }
}

/// Detect Claude Code status from pane content snapshot (first-seen pane only).
pub fn detect_status(content: &str) -> ClaudeCodeStatus {
    if is_waiting_for_input(content) {
        return ClaudeCodeStatus::WaitingInput;
    }
    if is_thinking(content) {
        return ClaudeCodeStatus::Thinking;
    }
    let working = content.contains("ctrl+c") && content.contains("to interrupt");
    if has_input_field(content) {
        return if working { ClaudeCodeStatus::Working } else { ClaudeCodeStatus::Idle };
    }
    if working {
        return ClaudeCodeStatus::Working;
    }
    ClaudeCodeStatus::Unknown
}

/// Detect status when content has NOT changed since last check.
pub fn detect_static_status(content: &str) -> ClaudeCodeStatus {
    if is_waiting_for_input(content) {
        return ClaudeCodeStatus::WaitingInput;
    }
    if is_thinking(content) {
        return ClaudeCodeStatus::Thinking;
    }
    if has_input_field(content) {
        return ClaudeCodeStatus::Idle;
    }
    ClaudeCodeStatus::Unknown
}

fn has_input_field(content: &str) -> bool {
    let lines: Vec<&str> = content.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.contains('❯') && i > 0 && lines[i - 1].contains('─') {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_when_ctrl_c_hint_and_input_field() {
        let content = "* processing (ctrl+c to interrupt)\n─────\n❯ hello";
        assert_eq!(detect_status(content), ClaudeCodeStatus::Working);
    }

    #[test]
    fn idle_when_input_field_no_interrupt() {
        let content = "● Done\n─────\n❯ hello";
        assert_eq!(detect_status(content), ClaudeCodeStatus::Idle);
    }

    #[test]
    fn waiting_input_on_yn_prompt() {
        let content = "Delete files? [y/n]";
        assert_eq!(detect_status(content), ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn waiting_input_tool_approval_footer() {
        let content = "Do you want to proceed?\n❯ 1. Yes\n  2. No\nEsc to cancel · Tab to amend · ctrl+e to explain";
        assert_eq!(detect_status(content), ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn waiting_input_static_tool_approval() {
        let content = "Do you want to proceed?\n❯ 1. Yes\n  2. No\nEsc to cancel · Tab to amend · ctrl+e to explain";
        assert_eq!(detect_static_status(content), ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn waiting_input_selection_dialog() {
        let content = "How do you want to handle this?\n❯ 1. Option A\n  2. Option B\nEnter to select · ↑/↓ to navigate · Esc to cancel";
        assert_eq!(detect_status(content), ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn waiting_input_even_when_tool_also_running() {
        let content = "Bash(find ...)\n└ Waiting…\nDo you want to proceed?\n❯ 1. Yes\nEsc to cancel · Tab to amend · ctrl+e to explain";
        assert_eq!(detect_status(content), ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn no_false_positive_from_partial_phrases() {
        // "ctrl+e to explain" alone (e.g. in a table I wrote) — should NOT trigger
        let content = "ctrl+e to explain or selection dialog visible | WaitingInput\n─────\n❯";
        assert_eq!(detect_status(content), ClaudeCodeStatus::Idle);
    }

    #[test]
    fn no_false_positive_from_esc_to_cancel_alone() {
        let content = "You can press Esc to cancel the operation if needed.\n─────\n❯";
        assert_eq!(detect_status(content), ClaudeCodeStatus::Idle);
    }

    #[test]
    fn working_when_content_changes_and_no_dialog() {
        let content = "* Processing files…\n(ctrl+c to interrupt)";
        assert_eq!(detect_changed_status(content), ClaudeCodeStatus::Working);
    }

    #[test]
    fn changed_content_with_spinner_is_thinking() {
        let content = "· Concocting… (1m 25s · ↓ 3.1k tokens)\n─────\n❯";
        assert_eq!(detect_changed_status(content), ClaudeCodeStatus::Thinking);
    }

    #[test]
    fn static_content_with_spinner_is_thinking() {
        let content = "✻ Reading file…\n─────\n❯";
        assert_eq!(detect_static_status(content), ClaudeCodeStatus::Thinking);
    }

    #[test]
    fn first_seen_spinner_is_thinking() {
        let content = "✶ Working…\n─────\n❯";
        assert_eq!(detect_status(content), ClaudeCodeStatus::Thinking);
    }

    #[test]
    fn all_spinner_chars_detected() {
        for &ch in &['\u{00B7}', '\u{273B}', '\u{273D}', '\u{2736}', '\u{2733}', '\u{2722}'] {
            let content = format!("{} Doing something…", ch);
            assert_eq!(detect_static_status(&content), ClaudeCodeStatus::Thinking,
                "spinner char U+{:04X} not detected", ch as u32);
        }
    }

    #[test]
    fn unknown_with_no_indicators() {
        let content = "some random terminal output";
        assert_eq!(detect_status(content), ClaudeCodeStatus::Unknown);
    }

    #[test]
    fn border_not_directly_above_prompt_is_unknown() {
        let content = "─────\nsome text\n❯ hello";
        assert_eq!(detect_status(content), ClaudeCodeStatus::Unknown);
    }

    #[test]
    fn static_waiting_input_on_yn() {
        let content = "Delete files? [y/n]";
        assert_eq!(detect_static_status(content), ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn static_idle_when_input_field() {
        let content = "● Done\n─────\n❯ hello";
        assert_eq!(detect_static_status(content), ClaudeCodeStatus::Idle);
    }
}
