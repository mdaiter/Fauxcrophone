use crate::{MixerStatus, get_mixer_status, set_source_gain_db, set_source_mute};

/// Fetch the current mixer status snapshot if the mixer is active.
pub fn get_status() -> Option<MixerStatus> {
    get_mixer_status()
}

/// Adjust the gain (in decibels) for the specified source.
pub fn set_gain(source_id: u32, gain_db: f32) -> bool {
    set_source_gain_db(source_id, gain_db)
}

/// Toggle the mute state for the specified source.
pub fn set_mute(source_id: u32, muted: bool) -> bool {
    set_source_mute(source_id, muted)
}
