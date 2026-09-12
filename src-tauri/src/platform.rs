//! Keeping the machine awake during a session.
//!
//! A forty-minute meditation is forty minutes of no keyboard or mouse, which
//! is exactly what the idle timer is watching for. Without this the display
//! sleeps and, on most settings, the machine follows and the audio stops.

/// Holds a system wake assertion for as long as it exists.
///
/// The display is deliberately allowed to sleep: a dark screen is welcome
/// during a session, and only the *system* needs to stay up to keep the audio
/// running.
#[derive(Default)]
pub struct StayAwake {
    handle: Option<keepawake::KeepAwake>,
    /// Why it could not be taken, if it could not. Surfaced rather than
    /// swallowed: a session that dies after ten minutes is baffling otherwise.
    pub last_error: Option<String>,
}

impl StayAwake {
    pub fn engage(&mut self) {
        if self.handle.is_some() {
            return;
        }
        match keepawake::Builder::default()
            .display(false)
            .idle(true)
            .reason("Playing a TrueZen session")
            .app_name("TrueZen")
            .app_reverse_domain("com.truezen.desktop")
            .create()
        {
            Ok(h) => {
                self.handle = Some(h);
                self.last_error = None;
            }
            Err(e) => self.last_error = Some(format!("could not keep the machine awake: {e}")),
        }
    }

    pub fn release(&mut self) {
        self.handle = None;
    }

    pub fn engaged(&self) -> bool {
        self.handle.is_some()
    }
}
