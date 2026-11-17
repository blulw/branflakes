//! Loudness analysis for volume normalization.
//!
//! Measures a song's integrated loudness (EBU R128 / ITU-R BS.1770: K-weighted,
//! gated mean square over 400 ms blocks) so playback can scale each track
//! towards a common target. Results are cached in `track_loudness`.

use crate::error::{AppError, AppResult};
use rusqlite::{params, Connection, OptionalExtension};
use std::io::Cursor;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Downloads larger than this are not analysed (≈ a very long mix).
pub const MAX_DOWNLOAD_BYTES: usize = 80 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 3],
}

impl Biquad {
    fn run(&self, st: &mut [f64; 2], x: f64) -> f64 {
        // Transposed direct form II.
        let y = self.b[0] * x + st[0];
        st[0] = self.b[1] * x - self.a[1] * y + st[1];
        st[1] = self.b[2] * x - self.a[2] * y;
        y
    }
}

/// BS.1770 K-weighting (high-shelf + RLB high-pass) for any sample rate.
fn k_weighting(rate: f64) -> (Biquad, Biquad) {
    let pi = std::f64::consts::PI;
    let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
    let k = (pi * f0 / rate).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
        a: [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    };
    let (f0, q) = (38.13547087602444, 0.5003270373238773);
    let k = (pi * f0 / rate).tan();
    let d = 1.0 + k / q + k * k;
    let hp = Biquad { b: [1.0, -2.0, 1.0], a: [1.0, 2.0 * (k * k - 1.0) / d, (1.0 - k / q + k * k) / d] };
    (shelf, hp)
}

/// Streaming integrated-loudness meter.
pub struct Meter {
    channels: usize,
    shelf: Biquad,
    hp: Biquad,
    state: Vec<[[f64; 2]; 2]>,
    weights: Vec<f64>,
    sub_len: usize,
    sub_pos: usize,
    sub_acc: f64,
    /// Weighted mean square of each 100 ms sub-block.
    subs: Vec<f64>,
}

impl Meter {
    pub fn new(rate: u32, channels: usize) -> Self {
        let (shelf, hp) = k_weighting(rate as f64);
        // BS.1770 channel weights: surrounds (L/R rear in 5.1) get +1.5 dB, LFE ignored.
        let weights = (0..channels)
            .map(|c| match (channels, c) {
                (6, 3) => 0.0,
                (6, 4) | (6, 5) => 1.41,
                _ => 1.0,
            })
            .collect();
        Meter {
            channels: channels.max(1),
            shelf,
            hp,
            state: vec![[[0.0; 2]; 2]; channels.max(1)],
            weights,
            sub_len: (rate as usize / 10).max(1),
            sub_pos: 0,
            sub_acc: 0.0,
            subs: Vec::new(),
        }
    }

    /// Feed interleaved samples.
    pub fn push(&mut self, interleaved: &[f32]) {
        for frame in interleaved.chunks_exact(self.channels) {
            let mut sum = 0.0;
            for (c, &x) in frame.iter().enumerate() {
                let w = self.weights.get(c).copied().unwrap_or(1.0);
                if w == 0.0 {
                    continue;
                }
                let st = &mut self.state[c];
                let y = self.shelf.run(&mut st[0], x as f64);
                let y = self.hp.run(&mut st[1], y);
                sum += w * y * y;
            }
            self.sub_acc += sum;
            self.sub_pos += 1;
            if self.sub_pos == self.sub_len {
                self.subs.push(self.sub_acc / self.sub_len as f64);
                self.sub_acc = 0.0;
                self.sub_pos = 0;
            }
        }
    }

    /// Integrated loudness in LUFS, or None for silence / too-short input.
    pub fn integrated(&self) -> Option<f64> {
        if self.subs.len() < 4 {
            return None;
        }
        // 400 ms blocks with 75 % overlap.
        let blocks: Vec<f64> = self.subs.windows(4).map(|w| w.iter().sum::<f64>() / 4.0).collect();
        let lufs = |z: f64| -0.691 + 10.0 * z.log10();
        let above = |thr: f64| -> Vec<f64> { blocks.iter().copied().filter(|&z| z > 0.0 && lufs(z) > thr).collect() };
        let abs: Vec<f64> = above(-70.0);
        if abs.is_empty() {
            return None;
        }
        let rel_gate = lufs(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
        let rel: Vec<f64> = abs.into_iter().filter(|&z| lufs(z) > rel_gate).collect();
        if rel.is_empty() {
            return None;
        }
        Some(lufs(rel.iter().sum::<f64>() / rel.len() as f64))
    }
}

/// Decode a whole media stream and measure its integrated loudness.
pub fn measure(source: Box<dyn MediaSource>, ext: Option<&str>) -> AppResult<Option<f64>> {
    let mut hint = Hint::new();
    if let Some(e) = ext {
        hint.with_extension(e);
    }
    let mss = MediaSourceStream::new(source, Default::default());
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| AppError::Unsupported(format!("cannot read audio for loudness analysis: {e}")))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| AppError::Unsupported("no audio track".into()))?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| AppError::Unsupported(format!("unsupported codec: {e}")))?;
    let mut meter: Option<Meter> = None;
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(_)) | Err(SymError::ResetRequired) => break,
            Err(e) => return Err(AppError::Other(format!("loudness analysis failed: {e}"))),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(_)) => continue,
            Err(SymError::IoError(_)) => break,
            Err(e) => return Err(AppError::Other(format!("loudness analysis failed: {e}"))),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count();
        let m = meter.get_or_insert_with(|| Meter::new(spec.rate, channels));
        if m.channels != channels.max(1) {
            continue;
        }
        let b = buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if (b.capacity() as u64) < decoded.capacity() as u64 * channels as u64 {
            *b = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        b.copy_interleaved_ref(decoded);
        m.push(b.samples());
    }
    Ok(meter.and_then(|m| m.integrated()))
}

pub fn measure_file(path: &std::path::Path) -> AppResult<Option<f64>> {
    let file = std::fs::File::open(path)?;
    let ext = path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase());
    measure(Box::new(file), ext.as_deref())
}

pub fn measure_bytes(bytes: Vec<u8>, ext: Option<&str>) -> AppResult<Option<f64>> {
    measure(Box::new(Cursor::new(bytes)), ext)
}

/// Download a SoundCloud stream (progressive or HLS) for analysis.
pub async fn download_stream(client: &reqwest::Client, url: &str, protocol: &str) -> AppResult<(Vec<u8>, Option<String>)> {
    async fn get(client: &reqwest::Client, url: &str) -> AppResult<Vec<u8>> {
        let resp = client
            .get(url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| AppError::Network { provider: "soundcloud".into(), message: crate::http::sanitize_reqwest_error(&e) })?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::Network { provider: "soundcloud".into(), message: crate::http::sanitize_reqwest_error(&e) })?;
        Ok(bytes.to_vec())
    }
    let ext_of = |u: &str| -> Option<String> {
        let path = u.split(['?', '#']).next()?;
        let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
        (ext.len() <= 4).then_some(ext)
    };
    if protocol != "hls" {
        let data = get(client, url).await?;
        return Ok((data, ext_of(url)));
    }
    let base = url::Url::parse(url).map_err(|e| AppError::Other(format!("bad stream URL: {e}")))?;
    let playlist = String::from_utf8_lossy(&get(client, url).await?).into_owned();
    let resolve = |u: &str| base.join(u.trim()).map(|u| u.to_string()).map_err(|e| AppError::Other(format!("bad segment URL: {e}")));
    let mut out = Vec::new();
    let mut ext = None;
    for line in playlist.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            if let Some(uri) = rest.split("URI=\"").nth(1).and_then(|s| s.split('"').next()) {
                let u = resolve(uri)?;
                ext = ext.or_else(|| ext_of(&u));
                out.extend(get(client, &u).await?);
            }
        } else if !line.is_empty() && !line.starts_with('#') {
            let u = resolve(line)?;
            ext = ext.or_else(|| ext_of(&u));
            out.extend(get(client, &u).await?);
        }
        if out.len() > MAX_DOWNLOAD_BYTES {
            return Err(AppError::Unsupported("stream too large to analyse".into()));
        }
    }
    // fMP4 segments: let the probe pick the container.
    if matches!(ext.as_deref(), Some("m4s") | Some("mp4")) {
        ext = Some("mp4".into());
    }
    Ok((out, ext))
}

/// Cached loudness. Outer None = not analysed yet; inner None = analysed but unmeasurable.
pub fn cached(conn: &Connection, track_id: &str, key: &str) -> AppResult<Option<Option<f64>>> {
    Ok(conn
        .query_row(
            "SELECT lufs FROM track_loudness WHERE track_id = ?1 AND source_key = ?2",
            params![track_id, key],
            |r| r.get::<_, Option<f64>>(0),
        )
        .optional()?)
}

pub fn store(conn: &Connection, track_id: &str, key: &str, lufs: Option<f64>) -> AppResult<()> {
    conn.execute(
        "INSERT INTO track_loudness (track_id, source_key, lufs, analyzed_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (track_id) DO UPDATE SET source_key = ?2, lufs = ?3, analyzed_at = ?4",
        params![track_id, key, lufs, crate::db::now_iso()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f64, amp: f64, secs: f64, channels: usize) -> Vec<f32> {
        let n = (rate as f64 * secs) as usize;
        let mut v = Vec::with_capacity(n * channels);
        for i in 0..n {
            let s = (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / rate as f64).sin()) as f32;
            for _ in 0..channels {
                v.push(s);
            }
        }
        v
    }

    /// EBU Tech 3341 reference: a 1 kHz stereo sine at -20 dBFS peak reads -20 LUFS.
    #[test]
    fn sine_1khz_matches_reference() {
        let amp = 10f64.powf(-20.0 / 20.0);
        let mut m = Meter::new(48000, 2);
        m.push(&sine(48000, 1000.0, amp, 5.0, 2));
        let l = m.integrated().unwrap();
        assert!((l - (-20.0)).abs() < 0.2, "got {l}");
    }

    #[test]
    fn quieter_signal_is_quieter_by_same_db() {
        let mut a = Meter::new(44100, 2);
        a.push(&sine(44100, 440.0, 0.5, 4.0, 2));
        let mut b = Meter::new(44100, 2);
        b.push(&sine(44100, 440.0, 0.25, 4.0, 2));
        let d = a.integrated().unwrap() - b.integrated().unwrap();
        assert!((d - 6.02).abs() < 0.05, "got {d}");
    }

    #[test]
    fn silence_is_unmeasurable() {
        let mut m = Meter::new(48000, 2);
        m.push(&vec![0.0; 48000 * 2 * 3]);
        assert!(m.integrated().is_none());
    }

    #[test]
    fn decodes_wav_bytes() {
        let rate = 22050u32;
        let samples = sine(rate, 1000.0, 0.1, 2.0, 1);
        let mut wav = Vec::new();
        let data_len = (samples.len() * 2) as u32;
        wav.extend(b"RIFF");
        wav.extend((36 + data_len).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(rate.to_le_bytes());
        wav.extend((rate * 2).to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(data_len.to_le_bytes());
        for s in samples {
            wav.extend(((s * 32767.0) as i16).to_le_bytes());
        }
        let l = measure_bytes(wav, Some("wav")).unwrap().unwrap();
        // mono 1 kHz at -20 dBFS peak → ≈ -23 LUFS.
        assert!((l - (-23.0)).abs() < 0.3, "got {l}");
    }
}
