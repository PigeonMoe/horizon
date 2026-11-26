//! Notifications exchanged by mods on one authoritative server.

pub trait Hooks {
    /// A mod published a notification to its own namespaced `topic`. The `payload_json`
    /// contains the exact JSON the mod submitted. A ready server delivers notifications in
    /// publication order at the end of its ticks, after the originating hook has returned.
    #[hook(
        side = "server",
        contract = "listen",
        host = "read",
        shared_event = "ServerModEvent"
    )]
    fn mod_event();
}
