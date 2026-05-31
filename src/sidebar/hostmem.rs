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
