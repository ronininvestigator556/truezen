//! Integration tests that need a real output device.
//!
//! Marked `#[ignore]` because CI runners usually have no audio hardware, where
//! these would fail for reasons unrelated to the code. Run them on a machine
//! with sound:
//!
//! ```text
//! cargo test --release -p truezen-host -- --ignored
//! ```
//!
//! They render at -46 dBFS, which exercises the whole signal path while
//! staying inaudible in practice.

use std::thread::sleep;
use std::time::Duration;

use truezen_engine::factory;
use truezen_engine::timeline::LayerParam;
use truezen_host::AudioHost;

/// Quiet enough to be inaudible, loud enough that the peak meter can prove
/// samples actually reached the device.
const TEST_GAIN: f64 = 0.005;

fn host_playing(preset_id: &str) -> AudioHost {
    let host = AudioHost::spawn(None).expect("no audio device available");
    host.load_preset(factory::by_id(preset_id).unwrap()).unwrap();
    host.set_master_gain(TEST_GAIN).unwrap();
    host.play().unwrap();
    host
}

#[test]
#[ignore = "requires an audio output device"]
fn enumerating_devices_succeeds() {
    let devices = AudioHost::devices().expect("enumeration failed");
    assert!(!devices.is_empty(), "no output devices found");
    // Ids are what get persisted in settings, so they must be present and
    // distinct even when two devices share a display name.
    let mut ids: Vec<&str> = devices.iter().map(|d| d.id.as_str()).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "duplicate device ids");
    assert!(devices.iter().all(|d| !d.id.is_empty()));
}

#[test]
#[ignore = "requires an audio output device"]
fn playback_advances_the_clock_and_reaches_the_device() {
    let host = host_playing("alpha-settle");
    sleep(Duration::from_millis(1500));

    let m = host.meters();
    let s = host.stats();

    assert!(s.callbacks > 0, "the audio callback never ran");
    assert!(
        m.position_s > 1.0 && m.position_s < 2.0,
        "clock read {:.3}s after 1.5s",
        m.position_s
    );
    assert!(m.playing, "transport did not report playing");
    assert!(m.peak_l > 0.0 && m.peak_r > 0.0, "no signal reached the output");
    assert!(
        (m.beat_hz - 10.0).abs() < 0.1,
        "expected ~10 Hz alpha, got {}",
        m.beat_hz
    );
}

#[test]
#[ignore = "requires an audio output device"]
fn pause_freezes_the_clock_and_stop_rewinds_it() {
    let host = host_playing("alpha-settle");
    sleep(Duration::from_millis(600));

    host.pause().unwrap();
    // Let the pause fade finish before sampling.
    sleep(Duration::from_millis(200));
    let paused_at = host.meters().position_s;
    sleep(Duration::from_millis(500));
    let still = host.meters().position_s;

    assert!(paused_at > 0.0, "clock never started");
    assert!(
        (still - paused_at).abs() < 0.02,
        "clock advanced {:.3}s while paused",
        still - paused_at
    );

    host.stop().unwrap();
    sleep(Duration::from_millis(200));
    assert!(host.meters().position_s < 0.01, "stop did not rewind");
}

#[test]
#[ignore = "requires an audio output device"]
fn seeking_lands_where_the_timeline_says_it_should() {
    let host = host_playing("deep-theta");
    sleep(Duration::from_millis(300));

    // deep-theta holds 4.5 Hz between t=1200s and t=2520s.
    host.seek(1500.0).unwrap();
    sleep(Duration::from_millis(300));

    let m = host.meters();
    assert!(m.position_s >= 1500.0, "seek did not take: {}", m.position_s);
    assert!(
        (m.beat_hz - 4.5).abs() < 0.05,
        "after seeking into the theta hold the beat read {} Hz",
        m.beat_hz
    );
}

/// Swapping presets hands the displaced layer stack back to the audio thread
/// for disposal. If that path were broken the callback would either drop
/// allocations itself or the queue would back up.
#[test]
#[ignore = "requires an audio output device"]
fn rapid_preset_swaps_keep_playing_and_drop_nothing() {
    let host = AudioHost::spawn(None).expect("no audio device available");
    host.set_master_gain(TEST_GAIN).unwrap();
    host.play().unwrap();

    for id in ["alpha-settle", "deep-theta", "sleep-onset", "gamma-concentration", "power-nap"] {
        host.load_preset(factory::by_id(id).unwrap()).unwrap();
        sleep(Duration::from_millis(250));
        assert!(host.meters().peak_l > 0.0, "{id} produced silence");
    }

    let s = host.stats();
    assert_eq!(s.dropped_commands, 0, "commands were dropped during swaps");
    assert_eq!(s.errors, 0, "stream errored during swaps");
    assert_eq!(s.overloads, 0, "callback overran during swaps");
}

/// The headline interaction: dialing a parameter while a session plays. A
/// burst far faster than any human drag must not overflow the queue.
#[test]
#[ignore = "requires an audio output device"]
fn a_burst_of_live_parameter_changes_is_not_dropped() {
    let host = host_playing("schumann-ground");
    sleep(Duration::from_millis(200));

    for i in 0..3_000 {
        let carrier = 120.0 + (i % 200) as f64;
        host.set_layer_param(0, LayerParam::Carrier, carrier).unwrap();
    }
    sleep(Duration::from_millis(400));

    let s = host.stats();
    assert_eq!(s.dropped_commands, 0, "dropped {} commands", s.dropped_commands);
    assert_eq!(s.overloads, 0, "callback overran while dialing");

    let m = host.meters();
    assert!(
        m.carrier_hz > 100.0 && m.carrier_hz < 340.0,
        "carrier ended at an implausible {} Hz",
        m.carrier_hz
    );
}

/// The visualiser feed must carry real samples, not zeros or stale data.
#[test]
#[ignore = "requires an audio output device"]
fn the_scope_carries_live_signal() {
    let host = host_playing("gamma-concentration");
    sleep(Duration::from_millis(800));

    let scope = host.scope();
    assert!(scope.wave.iter().all(|s| s.is_finite()));
    assert!(
        scope.wave.iter().any(|s| s.abs() > 0.0),
        "scope waveform was entirely silent"
    );
    assert!(
        scope.env.iter().any(|s| *s > 0.0),
        "scope envelope was entirely silent"
    );
}

/// Reported status drives user-facing warnings, so it must reflect reality.
#[test]
#[ignore = "requires an audio output device"]
fn status_describes_the_open_device() {
    let host = AudioHost::spawn(None).expect("no audio device available");
    let s = host.status();
    assert!(s.running, "stream not running: {:?}", s.last_error);
    assert!(s.device_name.is_some());
    assert!(s.sample_rate >= 8_000, "implausible rate {}", s.sample_rate);
    assert!(s.channels >= 1);
    assert_eq!(s.binaural_capable, s.channels >= 2);
    assert!(s.last_error.is_none());
}
