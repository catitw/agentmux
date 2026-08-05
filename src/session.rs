//! Session model: one unit of work = a work directory + a hermes tool name +
//! the process-layer status of the terminal running it.

use std::path::PathBuf;

/// A session is one unit of work: a work directory, a hermes tool, and the
/// embedded terminal running it.
pub struct Session {
    pub id: u64,
    pub work_dir: PathBuf,
    /// Display label, e.g. "Claude Code", "omp", "Shell".
    pub tool_name: String,
    /// Program to spawn in the terminal, e.g. "bash", "claude", "omp".
    pub command: String,
    pub status: SessionStatus,
}

/// Process-layer status of a session's terminal child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    /// Terminal child is alive (default right after spawn).
    Running,
    /// Child exited with status 0.
    Done,
    /// Child exited non-zero, or the terminal failed to spawn.
    Error,
}
