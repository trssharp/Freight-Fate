use super::*;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

/// Records what the engine tells it, and nothing else.
struct RecordingBackend {
    buses: Buses,
    log: Rc<RefCell<Vec<String>>>,
}

impl AudioBackend for RecordingBackend {
    fn name(&self) -> &'static str {
        "recording"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn buses(&self) -> &Buses {
        &self.buses
    }
    fn buses_mut(&mut self) -> &mut Buses {
        &mut self.buses
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn play_music(&mut self, track: &str, fade_ms: u32) {
        self.log
            .borrow_mut()
            .push(format!("music {track} {fade_ms}"));
    }
    fn start_loop(&mut self, channel: u32, key: &str, volume: f64, _fade_ms: u32) {
        self.log
            .borrow_mut()
            .push(format!("loop {channel} {key} {volume}"));
    }
    fn set_engine_voice_classic(&mut self, classic: bool) {
        self.log
            .borrow_mut()
            .push(format!("voice classic={classic}"));
    }
    fn set_volumes(&mut self, volumes: &VolumeUpdate) {
        self.buses.apply(volumes);
        self.log.borrow_mut().push("volumes".to_string());
    }
}

fn engine_with_pending_probe() -> (
    AudioEngine,
    std::sync::mpsc::Sender<Result<bool, bass_sys::safe::BassError>>,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut engine = AudioEngine::with_backend(Box::new(NullBackend::new()));
    engine.probe = Some(rx);
    engine.probe_headless = false;
    (engine, tx)
}

/// The requests boot makes while the device is still opening -- the menu
/// theme, the volume settings, the engine voice -- reach the real backend
/// the moment it arrives, and the null stand-in's "that asset does not
/// exist" answers do not outlive it.
#[test]
fn late_backend_hears_what_boot_asked_for() {
    let (mut engine, _tx) = engine_with_pending_probe();
    engine.play_music_with("menu_theme", 1500);
    engine.set_volumes(&VolumeUpdate::default().master(0.5));
    engine.set_engine_voice(true);
    // Cached against the null backend: false. Must not survive the swap.
    assert!(!engine.has_asset("ui/no_such_cue_for_this_test"));
    assert!(engine
        .asset_known
        .contains_key("ui/no_such_cue_for_this_test"));

    let log = Rc::new(RefCell::new(Vec::new()));
    engine.probe = None;
    engine.adopt_backend(
        Box::new(RecordingBackend {
            buses: Buses::new(),
            log: Rc::clone(&log),
        }),
        false,
    );

    let seen = log.borrow().join("; ");
    assert!(seen.contains("volumes"), "volumes not replayed: {seen}");
    assert!(
        seen.contains("voice classic=true"),
        "voice not replayed: {seen}"
    );
    assert!(
        seen.contains("music menu_theme 1500"),
        "music not replayed: {seen}"
    );
    assert!(
        engine.asset_known.is_empty(),
        "stand-in asset answers survived the swap"
    );
    assert!(!engine.take_silence_notice());
}

/// A probe that comes back empty-handed leaves the game on the null
/// backend and arms the notice, so the main menu can say out loud that
/// this run has no sound.
#[test]
fn failed_probe_runs_silent_and_says_so() {
    let (mut engine, tx) = engine_with_pending_probe();
    tx.send(Err(bass_sys::safe::BassError::NOT_LOADED)).unwrap();
    engine.update(0.0);
    assert!(engine.probe.is_none());
    assert_eq!(engine.backend_name(), "none");
    assert!(engine.take_silence_notice());
    assert!(!engine.take_silence_notice(), "the notice must speak once");
}

/// Music stopped before the backend arrives must not come back from the
/// dead when it does.
#[test]
fn stopped_music_is_not_replayed() {
    let (mut engine, _tx) = engine_with_pending_probe();
    engine.play_music_with("menu_theme", 1500);
    engine.stop_music_with(400);
    let log = Rc::new(RefCell::new(Vec::new()));
    engine.probe = None;
    engine.adopt_backend(
        Box::new(RecordingBackend {
            buses: Buses::new(),
            log: Rc::clone(&log),
        }),
        false,
    );
    assert!(
        !log.borrow().join("; ").contains("music"),
        "stopped music was resurrected"
    );
}

/// The main menu sets its ambience once, on entry, which at launch is
/// before the device has opened. The bed must start when the backend
/// arrives, not wait for the next screen change to ask again; a bed stopped
/// in the meantime stays stopped.
#[test]
fn late_backend_starts_the_loops_boot_left_running() {
    let (mut engine, _tx) = engine_with_pending_probe();
    engine.set_ambient(Some("ambient/menu_night_lot"));
    engine.start_loop_with(CH_WEATHER, "weather/rain", 0.4, 0);
    engine.set_loop_volume(CH_WEATHER, 0.7);
    engine.start_loop_with(CH_ROAD, "vehicle/road", 0.5, 0);
    engine.stop_loop_with(CH_ROAD, 0);

    let log = Rc::new(RefCell::new(Vec::new()));
    engine.probe = None;
    engine.adopt_backend(
        Box::new(RecordingBackend {
            buses: Buses::new(),
            log: Rc::clone(&log),
        }),
        false,
    );

    let seen = log.borrow().join("; ");
    assert!(
        seen.contains(&format!("loop {CH_AMBIENT} ambient/menu_night_lot 1")),
        "menu ambience not started: {seen}"
    );
    assert!(
        seen.contains(&format!("loop {CH_WEATHER} weather/rain 0.7")),
        "loop volume change lost: {seen}"
    );
    assert!(
        !seen.contains("vehicle/road"),
        "stopped loop was resurrected: {seen}"
    );
    assert!(engine.replay_loops.is_empty());
}
