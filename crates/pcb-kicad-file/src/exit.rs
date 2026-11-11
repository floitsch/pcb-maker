// Copyright (C) 2026 Toit contributors.

//! How a command's run exits: the exit codes (the verdict's `status`
//! decides which, `KiCadRunStatus::exit_code`), and the registry of the
//! errors that are the input's fault (`input_error`), which every reader
//! of a board, a project or a settings file marks so that the command
//! exits with `EXIT_USAGE`, not as an internal error.

use std::sync::Mutex;

/// The run finished and its result is complete.
pub const EXIT_COMPLETE: u8 = 0;
/// The run finished; the result is written but incomplete (connections
/// open, new DRC errors, constraints missed, parts unplaced).
pub const EXIT_INCOMPLETE: u8 = 1;
/// Bad arguments or input (missing or unreadable files, invalid JSON,
/// unknown keys or parts, an output directory that exists).
pub const EXIT_USAGE: u8 = 2;
/// The result is incomplete and the wall clock, not the work, cut the run
/// short: it depends on the machine's load.
pub const EXIT_CUT: u8 = 3;
/// An internal error or a panic.
pub const EXIT_INTERNAL: u8 = 4;

/// Errors marked as the input's fault (`input_error`), for the exit code.
static INPUT_ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Marks `message` as an error in the input (an unknown part in the
/// constraints, a board that does not parse): the command exits with
/// `EXIT_USAGE`, not as an internal error.
pub fn input_error(message: impl Into<String>) -> String {
    let message = message.into();
    if let Ok(mut errors) = INPUT_ERRORS.lock() {
        errors.push(message.clone());
    }
    message
}

/// What the input's error asks of the agent, noted where the error is
/// found (`note_input_action`), for the verdict's next_actions.
static INPUT_ACTIONS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Notes an action that fixes an input error about to be returned (the
/// outline's extent and the pitch of a lattice that does not fit): an
/// input error's verdict (`error_verdict`) lists it first.
pub fn note_input_action(action: impl Into<String>) {
    if let Ok(mut actions) = INPUT_ACTIONS.lock() {
        let action = action.into();
        if !actions.contains(&action) {
            actions.push(action);
        }
    }
}

/// The actions noted for input errors so far (`note_input_action`).
pub fn noted_input_actions() -> Vec<String> {
    INPUT_ACTIONS
        .lock()
        .map(|actions| actions.clone())
        .unwrap_or_default()
}

/// Whether `error` is, or wraps, a message marked by `input_error`.
pub fn is_input_error(error: &str) -> bool {
    INPUT_ERRORS
        .lock()
        .map(|errors| {
            errors
                .iter()
                .any(|message| !message.is_empty() && error.contains(message.as_str()))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marked_input_errors_are_recognised_when_wrapped() {
        let message = input_error("constraint names unknown part \"U99\"");
        assert!(is_input_error(&format!("placement: {message}")));
        assert!(!is_input_error("kicad-cli failed with exit status 1"));
    }
}
