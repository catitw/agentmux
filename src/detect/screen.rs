//! Grid region extraction (herdr-style `bottom_non_empty_lines`).
//!
//! The rule engine is deliberately fed plain `Vec<String>` regions so it
//! stays pure and unit-testable without a `Term`. This module is the only
//! place that touches alacritty's grid types.

use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::term::cell::Cell;

/// Default region size, mirroring herdr's `bottom_non_empty_lines(14)`.
pub const DEFAULT_BOTTOM_LINES: usize = 14;

/// Collect the bottom `max` non-empty lines of the grid as plain strings,
/// walking upward from the last visible line and skipping cleared rows.
///
/// This covers the visible viewport only (the alt screen used by TUI agents
/// has empty scrollback, so the visible screen is exactly what the agent
/// paints).
pub fn bottom_non_empty_lines(grid: &Grid<Cell>, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = grid.bottommost_line();
    loop {
        if out.len() >= max {
            break;
        }
        let row = &grid[line];
        if !row.is_clear() {
            let text: String = row.into_iter().map(|cell| cell.c).collect();
            let text = text.trim_end().to_string();
            if !text.trim().is_empty() {
                out.push(text);
            }
        }
        if line == grid.topmost_line() {
            break;
        }
        line -= 1;
    }
    out
}
