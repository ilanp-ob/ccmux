use crate::session::ClaudeCodeStatus;

/// Returns true for completion summary lines such as "✻ Brewed for 3m 30s" or
/// "* Baked for 6m 22s". These appear at the bottom of a finished session and
/// should be skipped when scanning backward for a conversational question.
fn is_completion_summary(line: &str) -> bool {
    const ORNAMENTS: &[char] = &[
        '\u{00B7}', // ·
        '\u{273B}', // ✻
        '\u{273D}', // ✽
        '\u{2736}', // ✶
        '\u{2733}', // ✳
        '\u{2722}', // ✢
        '*',
    ];
    matches!(line.chars().next(), Some(c) if ORNAMENTS.contains(&c)) && line.contains(" for ")
}

/// Returns true for plain numbered-option lines such as "1. Show me draft comment".
/// These appear when Claude presents a menu in conversational text (not a formal
/// dialog with a footer), and should be skipped to find the question above them.
fn is_numbered_option_line(line: &str) -> bool {
    let rest = line.trim_start_matches(|c: char| c.is_ascii_digit());
    // Must have consumed at least one digit and be followed by ". "
    rest.len() < line.len() && rest.starts_with(". ")
}

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
    let tail: Vec<&str> = content.lines().rev().take(12).collect();
    let tail_text = tail.join("\n");

    if tail_text.contains("[y/n]") || tail_text.contains("[Y/n]") {
        return true;
    }
    if tail_text.contains("Tab to amend · ctrl+e to explain") {
        return true;
    }
    // Subagent tool-approval footer (no "ctrl+e to explain" suffix)
    if tail_text.contains("Esc to cancel · Tab to amend") {
        return true;
    }
    // Covers "Enter to select · ↑/↓ to navigate" and
    // "Enter to select · Tab/Arrow keys to navigate · Esc to cancel"
    if tail_text.contains("Enter to select ·") {
        return true;
    }
    // Numbered selection dialogs (e.g. RTK tool approval) use "> N." or "❯ N."
    // as a cursor prefix without a standard footer line.
    if tail.iter().any(|line| {
        let t = line.trim();
        (t.starts_with("> ") || t.starts_with("\u{276F} "))
            && t.len() > 3
            && t.chars().nth(2).map(|c| c.is_ascii_digit()).unwrap_or(false)
            && t.chars().nth(3) == Some('.')
    }) {
        return true;
    }
    // Claude agents action items: "~ [N]: option | option"
    // These appear above the idle prompt after a run completes, so scan a wider
    // window than the tail used for footer-style dialogs.
    let wide: Vec<&str> = content.lines().rev().take(20).collect();
    if wide.iter().any(|line| {
        let t = line.trim();
        t.starts_with("~ [") && t.contains("]:")
    }) {
        return true;
    }
    // Conversational question: the last non-empty line above the most recent ─────\n❯
    // prompt ends with '?', meaning Claude asked a natural-language follow-up question.
    // Scan in reverse so we anchor to the bottom-most boundary, not stale scrollback.
    //
    // We skip over known "passthrough" line types that may appear between Claude's
    // question and the idle prompt without themselves being the question:
    //   • Completion summaries: "✻ Brewed for 3m 30s", "* Baked for 6m 22s"
    //   • Recap plugin lines:   "* recap: …" and their indented continuations
    //     (e.g. "  response (1-4). (disable recaps in /config)")
    //   • Plain numbered options: "1. Show me draft comment", "2. Fix locally"
    let lines_conv: Vec<&str> = content.lines().collect();
    for (i, line) in lines_conv.iter().enumerate().rev() {
        if line.contains('❯') && i > 0 && lines_conv[i - 1].contains('─') {
            for prev in lines_conv[..i - 1].iter().rev().take(20) {
                let t = prev.trim();
                if t.is_empty() { continue; }
                // "Interrupted · What should Claude do instead?" is Claude Code's
                // standard interrupt banner, not a genuine conversational question.
                if t.contains("Interrupted") { return false; }
                // Skip completion summary lines: start with ornament/asterisk + " for "
                // e.g. "✻ Brewed for 3m 30s", "* Baked for 6m 22s"
                if is_completion_summary(t) { continue; }
                // Skip recap plugin lines and their indented continuations
                // e.g. "* recap: Reviewing PR…" / "  response (1-4). (disable recaps…)"
                if t.starts_with("* recap:") || t.contains("disable recaps") { continue; }
                // Skip plain numbered option lines: "1. Show me draft comment"
                if is_numbered_option_line(t) { continue; }
                // A line beginning with the user's own input marker is the user's
                // message, not Claude asking a question — don't notify when the user
                // is the one who asked. Two markers exist:
                //   • '>'  (U+003E) — a sent message echoed in the transcript ("> well?")
                //   • '❯'  (U+276F) — a queued message shown while Claude is busy
                //     ("❯ did you use superpowers too?" above "Press up to edit
                //     queued messages"), or the idle input prompt itself.
                // (Numbered-selection cursors like "> 1." / "❯ 1." are handled
                // earlier and never reach here.)
                if t.starts_with('>') || t.starts_with('\u{276F}') { return false; }
                return t.ends_with('?');
            }
            break;
        }
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
        // Standard spinner: ornament char + space + active operation (always contains …).
        // Excludes completion summaries like "✻ Brewed for 3m 30s" which share the ornament.
        let mut chars = t.chars();
        matches!(chars.next(), Some(c) if SPINNERS.contains(&c))
            && chars.next() == Some(' ')
            && t.contains('\u{2026}')
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
    fn waiting_input_numbered_selection_no_footer() {
        // RTK tool approval dialog — no "Tab to amend" footer
        let content = "This command requires approval\nDo you want to proceed?\n> 1. Yes\n  2. Yes, and don't ask again for: rtk git *\n  3. No";
        assert_eq!(detect_status(content), ClaudeCodeStatus::WaitingInput);
        assert_eq!(detect_static_status(content), ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn waiting_input_agents_action_items() {
        let content = "or [b] Post note in #alerts\n---\nWhat would you like to do?\n~ [3]: d dismiss | b post Slack reply\n~ [5]: a comment on OPS-123\n---\n* Cogitated for 18m\n─────\n❯ ";
        assert_eq!(detect_status(content), ClaudeCodeStatus::WaitingInput);
        assert_eq!(detect_static_status(content), ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn waiting_input_subagent_dialog_with_extended_thinking() {
        // Subagent tool-approval footer ("Esc to cancel · Tab to amend", no ctrl+e)
        // pushed down by Smooshing spinner + separator + user message below the dialog
        let content = "how does it work in the real aws...\n\
            ─────\n\
            · Smooshing… (14m 57s · ↓ 2.9k tokens · thought for 15s)\n\
            Esc to cancel · Tab to amend\n\
            3. No\n\
            2. Yes, allow reading from api/\n\
            > 1. Yes\n\
            Do you want to proceed?";
        assert_eq!(detect_status(content), ClaudeCodeStatus::WaitingInput);
        assert_eq!(detect_static_status(content), ClaudeCodeStatus::WaitingInput);
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
    fn completion_summary_not_thinking() {
        // "✻ Brewed for 3m 30s" shares the ornament char but has no …
        let content = "✻ Brewed for 3m 30s\n─────\n❯";
        assert_eq!(detect_static_status(content), ClaudeCodeStatus::Idle);
        assert_eq!(detect_status(content), ClaudeCodeStatus::Idle);
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
    fn waiting_input_conversational_question() {
        let content = "Here's the Slack draft.\nWant me to adjust the tone?\n─────\n❯ ";
        assert_eq!(detect_status(content), ClaudeCodeStatus::WaitingInput);
        assert_eq!(detect_static_status(content), ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn waiting_input_conversational_question_with_blank_line_above_sep() {
        let content = "Want me to rerun with verbose output?\n\n─────\n❯ ";
        assert_eq!(detect_status(content), ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn no_false_positive_interrupted_banner() {
        // "Interrupted · What should Claude do instead?" ends with '?' but is not
        // a conversational question — it's Claude Code's standard interrupt banner.
        let content = "Interrupted · What should Claude do instead?\n─────\n❯ ";
        assert_eq!(detect_status(content), ClaudeCodeStatus::Idle);
        assert_eq!(detect_static_status(content), ClaudeCodeStatus::Idle);
    }

    #[test]
    fn no_false_positive_statement_above_prompt() {
        let content = "I have updated the file.\n─────\n❯ ";
        assert_eq!(detect_status(content), ClaudeCodeStatus::Idle);
    }

    #[test]
    fn no_false_positive_uses_bottom_most_boundary() {
        // Scrollback contains an old question, but the current prompt has a statement above it.
        let content = "Old question?\n─────\n❯ \nI updated the file.\n─────\n❯ ";
        assert_eq!(detect_status(content), ClaudeCodeStatus::Idle);
    }

    #[test]
    fn no_false_positive_user_authored_question() {
        // A line starting with '>' is the user's OWN message echoed above the prompt
        // (e.g. "> well?"). It must not be mistaken for Claude asking a question, so
        // it should not trigger a WaitingInput notification.
        let content = "> well?\n─────\n❯ ";
        assert_eq!(detect_status(content), ClaudeCodeStatus::Idle);
        assert_eq!(detect_static_status(content), ClaudeCodeStatus::Idle);
    }

    #[test]
    fn no_false_positive_queued_message_question() {
        // While Claude works, queued messages render with a '❯' (U+276F) prefix in a
        // box above "Press up to edit queued messages". A queued question ending in
        // '?' must NOT be read as Claude asking the user (real capture from a busy
        // session). Claude is "Garnishing…", so the status is Thinking, never Waiting.
        let content = "✳ Garnishing… (19m 50s · ↓ 44.8k tokens)\n\n  ❯ did you use superpowers too?\n──────── epic-15945-plan ──\n❯ Press up to edit queued messages\n────────\n  📁 ~/dev/houston-master1\n";
        assert_ne!(detect_status(content), ClaudeCodeStatus::WaitingInput);
        assert_ne!(detect_static_status(content), ClaudeCodeStatus::WaitingInput);
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

    // ── Numbered-option conversational prompts (code-review / recap style) ──

    #[test]
    fn waiting_input_numbered_options_with_completion_summary() {
        // Claude outputs a numbered-choice prompt, session ends with "* Baked for…"
        // No recap. The question above the options should still be detected.
        let content = "What would you like to do?\n\
            1. Show me draft comment\n\
            2. Fix locally\n\
            3. Already commented/handled\n\
            4. Unimportant or wrong — dismiss\n\
            \n\
            * Baked for 6m 22s\n\
            ─────\n\
            ❯ ";
        assert_eq!(detect_status(content), ClaudeCodeStatus::WaitingInput);
        assert_eq!(detect_static_status(content), ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn waiting_input_numbered_options_with_recap_and_summary() {
        // Real-world layout from the screenshot: numbered-choice prompt → completion
        // summary → recap plugin lines → idle prompt.  The recap injects
        // "  response (1-4). (disable recaps in /config)" above the separator.
        let content = "What would you like to do?\n\
            1. Show me draft comment\n\
            2. Fix locally\n\
            3. Already commented/handled\n\
            4. Unimportant or wrong — dismiss\n\
            \n\
            * Baked for 6m 22s\n\
            \n\
            * recap: Reviewing PR #11270, which fixes the latest attribute entry.\n\
            \u{0020}\u{0020}response (1-4). (disable recaps in /config)\n\
            ─────\n\
            ❯ ";
        assert_eq!(detect_status(content), ClaudeCodeStatus::WaitingInput);
        assert_eq!(detect_static_status(content), ClaudeCodeStatus::WaitingInput);
    }

    #[test]
    fn no_false_positive_numbered_list_no_question() {
        // Claude outputs a numbered summary without a preceding question — not a prompt.
        let content = "Here is what I changed:\n\
            1. Modified file A\n\
            2. Modified file B\n\
            * Baked for 2m\n\
            ─────\n\
            ❯ ";
        assert_eq!(detect_status(content), ClaudeCodeStatus::Idle);
    }
}
