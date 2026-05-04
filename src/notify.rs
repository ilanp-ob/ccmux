use std::collections::HashMap;
use std::time::Duration;

use crate::detection::detect_status;
use crate::session::ClaudeCodeStatus;
use crate::tmux::Tmux;

pub fn run(server: Option<String>) {
    let tmux = Tmux::new(server);
    let mut pane_status: HashMap<String, ClaudeCodeStatus> = HashMap::new();

    loop {
        // Exit cleanly when the tmux session is gone
        if tmux.current_session().map(|s| s.is_none()).unwrap_or(true) {
            break;
        }

        let Ok(out) = tmux.cmd()
            .args(["list-panes", "-aF",
                   "#{pane_id}\t#{pane_current_command}\t#{window_id}\t#{pane_active}"])
            .output()
        else {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };

        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let parts: Vec<&str> = line.splitn(4, '\t').collect();
            if parts.len() < 4 { continue; }

            let (pane_id, command, window_id, pane_active) =
                (parts[0], parts[1], parts[2], parts[3] == "1");

            if !command.contains("claude") && !command.contains("ocli") { continue; }

            let content = match tmux.capture_pane(pane_id, 30, true) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let new_status = detect_status(&content);
            let prev = pane_status.get(pane_id).cloned().unwrap_or(ClaudeCodeStatus::Unknown);

            if new_status == ClaudeCodeStatus::WaitingInput
                && prev != ClaudeCodeStatus::WaitingInput
                && !pane_active
            {
                let window_name = tmux.cmd()
                    .args(["display-message", "-t", pane_id, "-p", "#{window_name}"])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_else(|_| "Claude".to_string());

                fire_notification(&window_name);
                set_alert(&tmux, window_id, true);
            }

            if pane_active {
                set_alert(&tmux, window_id, false);
            }

            pane_status.insert(pane_id.to_string(), new_status);
        }

        std::thread::sleep(Duration::from_secs(2));
    }
}

fn fire_notification(window_name: &str) {
    let script = format!(
        "display notification {:?} with title \"ccmux\" subtitle \"Waiting for input\"",
        window_name
    );
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .status();
}

fn set_alert(tmux: &Tmux, window_id: &str, on: bool) {
    if on {
        let _ = tmux.cmd()
            .args(["set-window-option", "-t", window_id, "@ccmux_alert", "1"])
            .status();
    } else {
        let _ = tmux.cmd()
            .args(["set-window-option", "-ut", window_id, "@ccmux_alert"])
            .status();
    }
}
