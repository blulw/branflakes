//! Live loudness measurement for the in-app Spotify player.
//!
//! Spotify audio is DRM-protected, so it can't be decoded ahead of time like
//! local files. Instead, while a Spotify track plays on the in-app (Web
//! Playback SDK) device, this captures *this app's own* audio output with
//! Windows process loopback (Windows 10 2004+) and feeds it to the loudness
//! meter. Nothing is recorded or stored except the resulting LUFS number.
//!
//! The captured signal already includes the player volume, so the frontend
//! reports every volume change (`set_gain`) and samples are divided by it.
//! Audio right after a change is skipped while the new level settles.

use crate::loudness::Meter;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RATE: u32 = 48_000;
const CHANNELS: usize = 2;
/// Below this player volume the signal is too quiet to measure reliably.
const MIN_GAIN: f64 = 0.02;

struct Shared {
    meter: Meter,
    gain: f64,
    skip_until: Instant,
    error: Option<String>,
}

struct Session {
    track_id: String,
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub track_id: String,
    /// Seconds of audible audio measured so far.
    pub seconds: f64,
    /// Integrated loudness so far (LUFS).
    pub lufs: Option<f64>,
    /// Set when capture could not start or stopped with an error.
    pub error: Option<String>,
}

/// Start measuring `track_id`, replacing any running session. `gain` is the
/// player volume currently applied (0..1).
pub fn start(track_id: &str, gain: f64, settle: Duration) {
    let shared = Arc::new(Mutex::new(Shared {
        meter: Meter::new(RATE, CHANNELS),
        gain,
        skip_until: Instant::now() + settle,
        error: None,
    }));
    let stop = Arc::new(AtomicBool::new(false));
    {
        let (sh, st) = (shared.clone(), stop.clone());
        let spawned = std::thread::Builder::new().name("loudness-capture".into()).spawn(move || {
            if let Err(e) = run(&sh, &st) {
                log::warn!("loudness capture stopped: {e}");
                if let Ok(mut s) = sh.lock() {
                    s.error = Some(e);
                }
            }
        });
        if let Err(e) = spawned {
            shared.lock().unwrap().error = Some(format!("could not start capture thread: {e}"));
        }
    }
    *SESSION.lock().unwrap() = Some(Session { track_id: track_id.to_string(), stop, shared });
}

/// The player volume changed to `gain`; ignore audio for `settle` while it takes effect.
pub fn set_gain(gain: f64, settle: Duration) {
    if let Some(sess) = SESSION.lock().unwrap().as_ref() {
        let mut s = sess.shared.lock().unwrap();
        s.gain = gain;
        s.skip_until = s.skip_until.max(Instant::now() + settle);
    }
}

pub fn status() -> Option<CaptureStatus> {
    let guard = SESSION.lock().unwrap();
    let sess = guard.as_ref()?;
    let s = sess.shared.lock().unwrap();
    Some(CaptureStatus { track_id: sess.track_id.clone(), seconds: s.meter.audible_seconds(), lufs: s.meter.integrated(), error: s.error.clone() })
}

/// End the session; returns its final status.
pub fn stop() -> Option<CaptureStatus> {
    let st = status();
    SESSION.lock().unwrap().take();
    st
}

fn feed(shared: &Mutex<Shared>, samples: &[f32]) {
    let mut s = shared.lock().unwrap();
    if Instant::now() < s.skip_until || s.gain < MIN_GAIN {
        return;
    }
    let inv = (1.0 / s.gain) as f32;
    let scaled: Vec<f32> = samples.iter().map(|x| x * inv).collect();
    s.meter.push(&scaled);
}

#[cfg(windows)]
fn run(shared: &Mutex<Shared>, stop: &AtomicBool) -> Result<(), String> {
    use std::collections::VecDeque;
    use wasapi::{initialize_mta, AudioClient, Direction, SampleType, StreamMode, WaveFormat};

    let _ = initialize_mta();
    let format = WaveFormat::new(32, 32, SampleType::Float, RATE as usize, CHANNELS, None);
    let block = format.get_blockalign() as usize;
    // Include the process tree: WebView2's audio runs in child processes.
    let mut client = AudioClient::new_application_loopback_client(std::process::id(), true)
        .map_err(|e| format!("this version of Windows can't capture app audio ({e})"))?;
    client
        .initialize_client(&format, Direction::Capture, StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 }, None)
        .map_err(|e| format!("could not set up audio capture: {e}"))?;
    let event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
    let capture = client.get_audiocaptureclient().map_err(|e| e.to_string())?;
    let mut queue: VecDeque<u8> = VecDeque::new();
    client.start_stream().map_err(|e| e.to_string())?;
    let mut samples: Vec<f32> = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        // No events arrive while the app is silent: just keep checking `stop`.
        if event.wait_for_event(250).is_err() {
            continue;
        }
        loop {
            match capture.get_next_packet_size() {
                Ok(Some(n)) if n > 0 => {
                    capture.read_from_device_to_deque(&mut queue).map_err(|e| e.to_string())?;
                }
                Ok(_) => break,
                Err(e) => return Err(e.to_string()),
            }
        }
        let usable = queue.len() / block * block;
        if usable == 0 {
            continue;
        }
        samples.clear();
        let bytes: Vec<u8> = queue.drain(..usable).collect();
        samples.extend(bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
        feed(shared, &samples);
    }
    let _ = client.stop_stream();
    Ok(())
}

#[cfg(not(windows))]
fn run(_shared: &Mutex<Shared>, _stop: &AtomicBool) -> Result<(), String> {
    Err("live loudness capture is only available on Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compensates_for_player_volume_and_skips_settling_audio() {
        let shared = Mutex::new(Shared { meter: Meter::new(RATE, CHANNELS), gain: 0.5, skip_until: Instant::now(), error: None });
        // A full-scale-equivalent 1 kHz sine at -20 dBFS, played at volume 0.5.
        let amp = 0.1f32 * 0.5;
        let frames: Vec<f32> = (0..RATE as usize * 4)
            .flat_map(|i| {
                let v = amp * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / RATE as f32).sin();
                [v, v]
            })
            .collect();
        feed(&shared, &frames);
        let l = shared.lock().unwrap().meter.integrated().unwrap();
        assert!((l - (-20.0)).abs() < 0.2, "got {l}");
        // While settling, nothing is measured.
        shared.lock().unwrap().skip_until = Instant::now() + Duration::from_secs(60);
        let before = shared.lock().unwrap().meter.audible_seconds();
        feed(&shared, &frames);
        assert_eq!(shared.lock().unwrap().meter.audible_seconds(), before);
    }

    /// Plays a quiet (-40 dBFS) 1 kHz tone on the default device and measures
    /// it through process loopback. Needs real audio hardware:
    /// `cargo test capture_measures_own_output -- --ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn capture_measures_own_output() {
        use wasapi::*;
        let _ = initialize_mta();
        start("test", 1.0, Duration::from_millis(300));
        let device = DeviceEnumerator::new().unwrap().get_default_device(Direction::Render).unwrap();
        let mut client = device.get_iaudioclient().unwrap();
        let fmt = WaveFormat::new(32, 32, SampleType::Float, 48000, 2, None);
        let (def, _) = client.get_device_period().unwrap();
        client.initialize_client(&fmt, Direction::Render, StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: def }, None).unwrap();
        let ev = client.set_get_eventhandle().unwrap();
        let render = client.get_audiorenderclient().unwrap();
        let mut n = 0u64;
        let mut write = |frames: usize| {
            let mut data = Vec::with_capacity(frames * 8);
            for _ in 0..frames {
                let v = (0.01 * (2.0 * std::f64::consts::PI * 1000.0 * n as f64 / 48000.0).sin()) as f32;
                n += 1;
                data.extend(v.to_le_bytes());
                data.extend(v.to_le_bytes());
            }
            render.write_to_device(&data, None).unwrap();
        };
        write(client.get_available_space_in_frames().unwrap() as usize);
        client.start_stream().unwrap();
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(4) {
            ev.wait_for_event(1000).unwrap();
            write(client.get_available_space_in_frames().unwrap() as usize);
        }
        client.stop_stream().unwrap();
        let st = stop().unwrap();
        println!("captured {:.1}s, {:?} LUFS, error {:?}", st.seconds, st.lufs, st.error);
        assert!(st.error.is_none());
        assert!(st.seconds > 2.0);
        assert!((st.lufs.unwrap() - (-40.0)).abs() < 1.0);
    }
}
