//! Admission and adjustments inside a stock furnace tick.

pub trait Hooks {
    /// Adjust a stock furnace, blast furnace or smoker's actual processing.
    /// `entity` identifies the loaded block entity and `game_time` is its level's signed tick.
    /// Fuel is offered only when it will be consumed. Result is offered only once the
    /// vanilla recipe can complete, before writing output, consuming input or recording XP.
    /// Progress is offered after the native step while an operation is in progress.
    /// The stage cannot be replaced with another kind. Only `duration`, `result` and
    /// `elapsed` respectively are read back. Nonpositive durations and out-of-range
    /// progress are ignored. An absent or incompatible result leaves input and XP intact.
    #[hook(
        side = "server",
        host = "read",
        contract = "collect",
        out = "stage",
        paper = "FurnaceBurnEvent, FurnaceSmeltEvent",
        neoforge = "none"
    )]
    fn furnace_process();
}
