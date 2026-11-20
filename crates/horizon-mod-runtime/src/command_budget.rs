//! The native dispatcher's admission of a longer deadline for console administration.

use std::cell::Cell;

thread_local! {
    static ADMINISTRATIVE_COMMAND: Cell<bool> = const { Cell::new(false) };
}

/// The dispatcher scopes this to an already parsed mod command. Only an actual console
/// or RCON owner executing an operator-only usage can request the administrative budget.
pub fn with_run_mod_command<R>(
    external_console: bool,
    source_permission: u8,
    execution_permission: u8,
    run: impl FnOnce() -> R,
) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            ADMINISTRATIVE_COMMAND.set(self.0);
        }
    }

    let admitted =
        external_console && source_permission == 4 && (2..=4).contains(&execution_permission);
    let _restore = Restore(ADMINISTRATIVE_COMMAND.replace(admitted));
    run()
}

/// Whether the command being dispatched was admitted for a longer budget.
pub fn administrative() -> bool {
    ADMINISTRATIVE_COMMAND.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_owner_console_operator_commands_receive_authority_and_each_dispatch_rechecks_it() {
        assert!(!administrative());
        for (origin, source, required) in [(false, 4, 2), (true, 2, 2), (true, 4, 0), (true, 4, 1)]
        {
            with_run_mod_command(origin, source, required, || assert!(!administrative()));
        }
        with_run_mod_command(true, 4, 2, || {
            assert!(administrative());
            with_run_mod_command(false, 4, 2, || assert!(!administrative()));
            assert!(administrative());
        });
        assert!(!administrative());
    }
}
