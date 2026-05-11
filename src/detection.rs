use crate::session::ClaudeCodeStatus;

/// Returns true when the pane content shows a Claude confirmation/permission
/// dialog that requires the user to respond before Claude can continue.
fn is_waiting_for_input(content: &str) -> bool {
    // Legacy shell-style prompts
    if content.contains("[y/n]") || content.contains("[Y/n]") {
        return true;
    }
    // Claude's numbered-choice dialogs: "Esc to cancel" always appears in the
    // footer, and "Do you want to proceed?" is the standard permission phrasing.
    if content.contains("Esc to cancel") {
        return true;
    }
    if content.contains("Do you want to proceed?") {
        return true;
    }
    false
}

/// Detect Claude Code status from pane content snapshot.
pub fn detect_status(content: &str) -> ClaudeCodeStatus {
    // Confirmation dialogs take priority — user must respond regardless of
    // whether a background tool is also running.
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
    fn waiting_input_on_claude_confirmation_dialog() {
        let content = "Do you want to proceed?\n❯ 1. Yes\n  2. No\nEsc to cancel · Tab to amend";
        assert_eq!(detect_status(content), ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn waiting_input_static_on_esc_to_cancel() {
        let content = "Do you want to proceed?\n❯ 1. Yes\n  2. No\nEsc to cancel · Tab to amend";
        assert_eq!(detect_static_status(content), ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn waiting_input_even_when_tool_also_running() {
        // Claude shows a permission dialog while a background bash tool is still running
        let content = "Bash(find ...)\n└ Waiting…\nDo you want to proceed?\n❯ 1. Yes\nEsc to cancel · Tab to amend · ctrl+c to interrupt";
        assert_eq!(detect_status(content), ClaudeCodeStatus::WaitingInput);
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
