//! Live audio soak test.
//!
//! Verifies on real hardware what the offline tests cannot: that the callback
//! keeps its deadline, that dialing parameters during playback neither
//! glitches nor drops commands, and that the session clock advances correctly
//! over a long run.
//!
//! Exits non-zero if the stream reported an error, overran its deadline, or
//! dropped a command -- so it is usable as a CI gate on a machine with audio.

use std::time::{Duration, Instant};

use clap::Parser;
use truezen_engine::factory;
use truezen_engine::timeline::LayerParam;
use truezen_host::AudioHost;

#[derive(Parser)]
#[command(about = "Soak-test the TrueZen audio host on real hardware")]
struct Cli {
    /// Factory preset to play.
    #[arg(short, long, default_value = "alpha-settle")]
    preset: String,
    #[arg(short, long, default_value_t = 20.0)]
    seconds: f64,
    /// Master gain. The default is deliberately near-silent: this is a
    /// correctness check, not a listening session.
    #[arg(short, long, default_value_t = 0.03)]
    gain: f64,
    /// Continuously dial the beat frequency, exercising the live-control path
    /// under load.
    #[arg(long)]
    sweep: bool,
    /// List output devices and exit.
    #[arg(long)]
    list_devices: bool,
    /// Output device id (see --list-devices). Defaults to the system default.
    #[arg(long)]
    device: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    if cli.list_devices {
        for d in AudioHost::devices()? {
            let mark = if d.is_default { "*" } else { " " };
            println!("{mark} {:<40} {}", d.name, d.id);
        }
        return Ok(());
    }

    let preset = factory::by_id(&cli.preset)
        .ok_or_else(|| format!("no factory preset '{}'", cli.preset))?;

    let host = AudioHost::spawn(cli.device)?;
    let status = host.status();
    println!(
        "device : {}\nformat : {} Hz, {} ch{}\npreset : {} ({})\n",
        status.device_name.as_deref().unwrap_or("<none>"),
        status.sample_rate,
        status.channels,
        if status.binaural_capable { "" } else { "  [MONO - binaural will not work]" },
        preset.name,
        preset.id,
    );

    host.load_preset(preset)?;
    host.set_master_gain(cli.gain)?;
    host.play()?;

    println!(
        "{:>6}  {:>8}  {:>9}  {:>10}  {:>9}  {:>7}  {:>6}",
        "t", "pos", "beat", "carrier", "peak L/R", "load", "cbs"
    );

    let started = Instant::now();
    let mut last_report = 0.0;
    let mut sweeps = 0u64;

    while started.elapsed().as_secs_f64() < cli.seconds {
        let t = started.elapsed().as_secs_f64();

        if cli.sweep {
            // A continuous dial, far faster than a human drag, to prove the
            // command path keeps up and the smoothers stay artefact-free.
            //
            // The carrier rather than the beat: most presets automate the
            // beat, and the timeline reasserts its value on every automation
            // block, so a manual beat set would be silently overwritten.
            let carrier = 220.0 + 80.0 * (t * 0.7).sin();
            host.set_layer_param(0, LayerParam::Carrier, carrier)?;
            sweeps += 1;
        }

        if t - last_report >= 1.0 {
            last_report = t;
            let m = host.meters();
            let s = host.stats();
            println!(
                "{t:>6.1}  {:>8.2}  {:>7.3} Hz  {:>7.2} Hz  {:>4.3}/{:<4.3}  {:>6.1}%  {:>6}",
                m.position_s,
                m.beat_hz,
                m.carrier_hz,
                m.peak_l,
                m.peak_r,
                s.last_load_permille as f64 / 10.0,
                s.callbacks,
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    // Read the session clock before stopping -- Stop rewinds the position to
    // zero, which would make the drift check meaningless.
    let m = host.meters();
    let elapsed = started.elapsed().as_secs_f64();

    host.stop()?;
    // Let the transport fade complete before tearing the stream down.
    std::thread::sleep(Duration::from_millis(200));

    let s = host.stats();
    println!("\n--- results ---");
    println!("callbacks         {}", s.callbacks);
    println!("overloads         {}   (callback used >80% of its deadline)", s.overloads);
    println!("stream errors     {}", s.errors);
    println!("dropped commands  {}", s.dropped_commands);
    println!("peak load         {:.1}%", s.max_load_permille as f64 / 10.0);
    if cli.sweep {
        println!("live param sets   {sweeps}");
    }

    // The session clock is driven by sample count, so it should track
    // wall-clock closely. A large gap means the stream stalled.
    let drift = m.position_s - elapsed;
    println!("clock drift       {drift:+.3}s vs wall clock");

    let mut failed = false;
    if s.callbacks == 0 {
        println!("\nFAIL: the audio callback never ran");
        failed = true;
    }
    if s.errors > 0 {
        println!("\nFAIL: the stream reported {} error(s)", s.errors);
        failed = true;
    }
    if s.overloads > 0 {
        println!("\nFAIL: {} callback(s) overran the deadline", s.overloads);
        failed = true;
    }
    if s.dropped_commands > 0 {
        println!("\nFAIL: {} command(s) were dropped", s.dropped_commands);
        failed = true;
    }
    if drift.abs() > 0.5 {
        println!("\nFAIL: session clock drifted {drift:+.3}s from wall clock");
        failed = true;
    }

    if failed {
        std::process::exit(1);
    }
    println!("\nPASS");
    Ok(())
}
