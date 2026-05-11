use crate::session::ClaudeCodeStatus;

/// Returns true when the pane content shows a Claude confirmation/permission
/// dialog that requires the user to respond before Claude can continue.
///
/// Dialog footers always render at the bottom of the terminal. We only check
/// the last few lines so code/text that *mentions* these phrases mid-output
/// (e.g. a code diff displaying the detection source itself) doesn't trigger.
fn is_waiting_for_input(content: &str) -> bool {
    // Legacy shell-style prompts — these are usually on a single line at the
    // bottom so full-content scan is fine; they're rarely in Claude output text.
    if content.contains("[y/n]") || content.contains("[Y/n]") {
        return true;
    }
    // Only scan the tail of the capture for Claude's dialog footer phrases.
    // Actual dialogs appear at the bottom; scrollback content (including code
    // that contains these strings as literals) is higher up.
    let tail: Vec<&str> = content.lines().rev().take(6).collect();
    let tail_text = tail.join("\n");
    if tail_text.contains("Tab to amend · ctrl+e to explain") {
        return true;
    }
    if tail_text.contains("Enter to select · ↑/↓ to navigate") {
        return true;
    }
    false
}

/// Status when content has CHANGED since last tick.
/// Safe assumption is Working; only override if a confirmation dialog is visible.
pub fn detect_changed_status(content: &str) -> ClaudeCodeStatus {
    if is_waiting_for_input(content) {
        ClaudeCodeStatus::WaitingInput
    } else {
        ClaudeCodeStatus::Working
    }
}

/// Detect Claude Code status from pane content snapshot (first-seen pane only).
pub fn detect_status(content: &str) -> ClaudeCodeStatus {
    if is_waiting_for_input(content) {
        return ClaudeCodeStatus::WaitingInput;
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
/// Only distinguishes WaitingInput from Idle (Working is determined externally).
pub fn detect_static_status(content: &str) -> ClaudeCodeStatus {
    if is_waiting_for_input(content) {
        return ClaudeCodeStatus::WaitingInput;
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
    fn changed_content_with_spinner_is_working() {
        // Spinner "· Concocting…" with no ctrl+c hint — still Working when content changes
        let content = "· Concocting… (1m 25s · ↓ 3.1k tokens)\n─────\n❯";
        assert_eq!(detect_changed_status(content), ClaudeCodeStatus::Working);
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
