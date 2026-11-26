//! The source's owned transfer profile snapshot hook.

pub trait Hooks {
    /// Capture the package's complete section for an active player. The server
    /// closed the player's menus and completed pending saves before this call.
    /// The snapshot runs between ticks, before the source ownership fence is
    /// published. Read through the supplied server host and return bytes for
    /// exactly this `id` and `schema`. Leaving the reply empty refuses export.
    #[hook(
        side = "server",
        host = "server_read",
        contract = "listen",
        owner = "mod_id"
    )]
    fn player_transfer_profile_capture(
        mod_id: &str,
        owner: u128,
        id: &Identifier,
        schema: u32,
        reply: &mut Option<Result<Vec<u8>, String>>,
    );
}
