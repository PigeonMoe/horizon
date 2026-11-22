//! The destination's owned, read-only transfer profile validation hook.

pub trait Hooks {
    /// Validate this package's required section before a held player is imported.
    /// Read `payload` according to its `id` and `schema`; validate it for the
    /// given `owner`. The candidate compound contains only this package's
    /// `PlayerPersisted` entries. Answer with this package's complete projected
    /// entries, or an error. Leaving the reply empty refuses the import.
    /// The hook receives immutable server services. Its reply is staged and
    /// cannot activate until the destination durably writes the player's file.
    #[hook(
        side = "server",
        host = "server_read",
        contract = "listen",
        owner = "mod_id"
    )]
    fn player_transfer_profile_stage(
        mod_id: &str,
        owner: u128,
        id: &Identifier,
        schema: u32,
        payload: &[u8],
        candidate: &Compound,
        reply: &mut Option<Result<Compound, String>>,
    );
}
