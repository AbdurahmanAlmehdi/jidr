//! WAV and raw PCM clips for microphone injection (#70).
//!
//! The emulator's `injectAudio` takes unsigned 8-bit or signed 16-bit
//! little-endian PCM, mono or stereo (`AudioFormat` in
//! `emulator_controller.proto`). WAV files in those formats pass through as
//! they are; 24- and 32-bit integer and 32/64-bit float WAVs are converted
//! to 16-bit. Sample rates outside 8–48 kHz are refused: the proto notes that
//! Android doesn't use rates above 48 kHz and that the emulator rejects rates
//! that are too high or too low, and Jidr doesn't resample.
//!
//! The parser follows the RIFF/WAVE layout (chunks of a four-byte id, a
//! little-endian `u32` size and the data padded to an even length; `fmt `
//! holds `WAVEFORMATEX`, `WAVE_FORMAT_EXTENSIBLE` carries the real format
//! code in the first two bytes of its sub-format GUID).

use std::time::Duration;

use jidr_core::{JidrError, JidrResult};

/// Lowest sample rate accepted, in Hz.
pub const MIN_RATE: u32 = 8_000;
/// Highest sample rate accepted, in Hz.
pub const MAX_RATE: u32 = 48_000;
/// Largest clip read into memory (about 23 minutes of 48 kHz 16-bit stereo).
pub const MAX_BYTES: u64 = 256 * 1024 * 1024;

const PCM: u16 = 1;
const FLOAT: u16 = 3;
const EXTENSIBLE: u16 = 0xFFFE;

/// The sample formats `injectAudio` accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    /// Unsigned 8-bit.
    U8,
    /// Signed 16-bit, little-endian.
    S16,
}

impl SampleFormat {
    pub fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::S16 => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::U8 => "u8",
            Self::S16 => "s16le",
        }
    }
}

/// Audio ready to inject: interleaved samples in a format the emulator takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clip {
    /// Samples per second, per channel.
    pub rate: u32,
    /// 1 (mono) or 2 (stereo).
    pub channels: u16,
    pub format: SampleFormat,
    /// Whole frames only.
    pub data: Vec<u8>,
    /// The file's own format when it was converted to 16-bit, e.g. `"pcm24"`.
    pub converted_from: Option<&'static str>,
}

impl Clip {
    /// Bytes per frame (one sample for every channel).
    pub fn frame_bytes(&self) -> usize {
        self.format.bytes() * usize::from(self.channels)
    }

    pub fn frames(&self) -> u64 {
        (self.data.len() / self.frame_bytes().max(1)) as u64
    }

    pub fn duration(&self) -> Duration {
        frames_duration(self.frames(), self.rate)
    }

    /// Bytes in a packet of `ms` milliseconds: whole frames, at least one.
    pub fn packet_bytes(&self, ms: u32) -> usize {
        let frames = (u64::from(self.rate) * u64::from(ms) / 1000).max(1);
        usize::try_from(frames)
            .unwrap_or(usize::MAX / 8)
            .saturating_mul(self.frame_bytes())
    }
}

/// How long `frames` frames last at `rate` Hz.
pub fn frames_duration(frames: u64, rate: u32) -> Duration {
    if rate == 0 {
        return Duration::ZERO;
    }
    let rate = u64::from(rate);
    Duration::from_secs(frames / rate)
        + Duration::from_nanos((frames % rate) * 1_000_000_000 / rate)
}

fn check_layout(rate: u32, channels: u16) -> JidrResult<()> {
    if !(MIN_RATE..=MAX_RATE).contains(&rate) {
        return Err(JidrError::usage(format!(
            "the sample rate is {rate} Hz; the emulator's microphone takes {MIN_RATE}–{MAX_RATE} Hz (convert it first, e.g. `afconvert -d LEI16@44100 in.wav out.wav`)"
        )));
    }
    if !(1..=2).contains(&channels) {
        return Err(JidrError::usage(format!(
            "the audio has {channels} channels; the emulator's microphone takes mono or stereo"
        )));
    }
    Ok(())
}

/// True when `bytes` start like a RIFF/WAVE file.
pub fn is_wav(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE"
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at + 2)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

#[derive(Debug, Clone, Copy)]
struct Fmt {
    code: u16,
    channels: u16,
    rate: u32,
    block_align: u16,
    bits: u16,
}

fn parse_fmt(c: &[u8]) -> JidrResult<Fmt> {
    let bad = || JidrError::usage("the WAV file's fmt chunk is too short");
    let mut code = u16_at(c, 0).ok_or_else(bad)?;
    let channels = u16_at(c, 2).ok_or_else(bad)?;
    let rate = u32_at(c, 4).ok_or_else(bad)?;
    let block_align = u16_at(c, 12).ok_or_else(bad)?;
    let bits = u16_at(c, 14).ok_or_else(bad)?;
    if code == EXTENSIBLE {
        // cbSize, valid bits, channel mask, then the sub-format GUID whose
        // first two bytes are the format code.
        code = u16_at(c, 24)
            .ok_or_else(|| JidrError::usage("the WAV file's extensible fmt chunk is too short"))?;
    }
    Ok(Fmt {
        code,
        channels,
        rate,
        block_align,
        bits,
    })
}

/// Parses a RIFF/WAVE file.
pub fn parse_wav(bytes: &[u8]) -> JidrResult<Clip> {
    if !is_wav(bytes) {
        return Err(JidrError::usage("not a WAV file (no RIFF/WAVE header)"));
    }
    let mut fmt: Option<Fmt> = None;
    let mut at = 12usize;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(bytes, at + 4).unwrap_or(0) as usize;
        let body = at + 8;
        // Streams written without knowing their length leave the size at
        // 0 or 0xFFFFFFFF; the data runs to the end of the file.
        let end = body.saturating_add(size).min(bytes.len());
        match id {
            b"fmt " => fmt = Some(parse_fmt(&bytes[body..end])?),
            b"data" => {
                let f = fmt.ok_or_else(|| {
                    JidrError::usage("the WAV file has no fmt chunk before its data")
                })?;
                let end = if size == 0 { bytes.len() } else { end };
                return convert(f, &bytes[body..end]);
            }
            _ => {}
        }
        at = end.saturating_add(size & 1);
        if end >= bytes.len() {
            break;
        }
    }
    Err(JidrError::usage("the WAV file has no data chunk"))
}

fn unsupported(f: Fmt) -> JidrError {
    let what = match f.code {
        PCM => format!("{}-bit PCM", f.bits),
        FLOAT => format!("{}-bit float", f.bits),
        2 => "ADPCM".into(),
        6 => "A-law".into(),
        7 => "µ-law".into(),
        0x55 => "MP3".into(),
        c => format!("format 0x{c:04x}"),
    };
    JidrError::usage(format!(
        "the WAV file is {what}; Jidr injects 8-, 16-, 24- or 32-bit PCM and 32- or 64-bit float WAVs (convert it first, e.g. `afconvert -f WAVE -d LEI16 in out.wav`)"
    ))
    .with_data(serde_json::json!({ "format_code": f.code, "bits": f.bits }))
}

fn convert(f: Fmt, data: &[u8]) -> JidrResult<Clip> {
    check_layout(f.rate, f.channels)?;
    let bytes_per_sample = usize::from(f.bits.div_ceil(8));
    let (format, converted_from, out) = match (f.code, f.bits) {
        (PCM, 8) => (SampleFormat::U8, None, data.to_vec()),
        (PCM, 16) => (SampleFormat::S16, None, data.to_vec()),
        (PCM, 24 | 32) => (
            SampleFormat::S16,
            Some(if f.bits == 24 { "pcm24" } else { "pcm32" }),
            // Little-endian: the top two bytes of each sample are its high 16 bits.
            data.chunks_exact(bytes_per_sample)
                .flat_map(|s| [s[bytes_per_sample - 2], s[bytes_per_sample - 1]])
                .collect(),
        ),
        (FLOAT, 32) => (
            SampleFormat::S16,
            Some("float32"),
            data.as_chunks::<4>()
                .0
                .iter()
                .flat_map(|s| float_to_s16(f64::from(f32::from_le_bytes(*s))))
                .collect(),
        ),
        (FLOAT, 64) => (
            SampleFormat::S16,
            Some("float64"),
            data.as_chunks::<8>()
                .0
                .iter()
                .flat_map(|s| float_to_s16(f64::from_le_bytes(*s)))
                .collect(),
        ),
        _ => return Err(unsupported(f)),
    };
    if usize::from(f.block_align) != bytes_per_sample * usize::from(f.channels) {
        return Err(JidrError::usage(format!(
            "the WAV file's block align ({}) doesn't match {} channel(s) of {}-bit samples",
            f.block_align, f.channels, f.bits
        )));
    }
    finish(f.rate, f.channels, format, out, converted_from)
}

fn float_to_s16(v: f64) -> [u8; 2] {
    let v = if v.is_nan() { 0.0 } else { v.clamp(-1.0, 1.0) };
    ((v * 32767.0).round() as i16).to_le_bytes()
}

fn finish(
    rate: u32,
    channels: u16,
    format: SampleFormat,
    mut data: Vec<u8>,
    converted_from: Option<&'static str>,
) -> JidrResult<Clip> {
    let frame = format.bytes() * usize::from(channels);
    data.truncate(data.len() - data.len() % frame);
    if data.is_empty() {
        return Err(JidrError::usage("the audio file holds no samples"));
    }
    Ok(Clip {
        rate,
        channels,
        format,
        data,
        converted_from,
    })
}

/// Headerless signed 16-bit little-endian PCM at `rate` Hz.
pub fn raw_s16(data: Vec<u8>, rate: u32, channels: u16) -> JidrResult<Clip> {
    check_layout(rate, channels)?;
    finish(rate, channels, SampleFormat::S16, data, None)
}

/// Reads a clip: a WAV file, or raw 16-bit PCM when `raw` gives its
/// `(sample_rate, channels)`.
pub fn load(path: &std::path::Path, raw: Option<(u32, u16)>) -> JidrResult<Clip> {
    let cannot =
        |e: std::io::Error| JidrError::usage(format!("cannot read {}: {e}", path.display()));
    let len = std::fs::metadata(path).map_err(cannot)?.len();
    if len > MAX_BYTES {
        return Err(JidrError::usage(format!(
            "{} is {} MB; Jidr injects files up to {} MB",
            path.display(),
            len / (1024 * 1024),
            MAX_BYTES / (1024 * 1024)
        )));
    }
    let bytes = std::fs::read(path).map_err(cannot)?;
    match raw {
        None if is_wav(&bytes) => parse_wav(&bytes),
        None => Err(JidrError::usage(format!(
            "{} isn't a WAV file; for headerless 16-bit PCM pass its sample rate (and channels)",
            path.display()
        ))),
        Some(_) if is_wav(&bytes) => Err(JidrError::usage(format!(
            "{} is a WAV file; its header gives the format, so don't pass a sample rate or channels",
            path.display()
        ))),
        Some((rate, channels)) => raw_s16(bytes, rate, channels),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A WAV file with a `fmt ` chunk of `fmt_extra` extra bytes, an
    /// unknown `LIST` chunk, and `data`.
    pub fn wav(code: u16, channels: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
        let align = channels * bits.div_ceil(8);
        let mut fmt = Vec::new();
        fmt.extend(code.to_le_bytes());
        fmt.extend(channels.to_le_bytes());
        fmt.extend(rate.to_le_bytes());
        fmt.extend((rate * u32::from(align)).to_le_bytes());
        fmt.extend(align.to_le_bytes());
        fmt.extend(bits.to_le_bytes());
        let mut out = Vec::new();
        out.extend(b"RIFF");
        out.extend(0u32.to_le_bytes());
        out.extend(b"WAVE");
        out.extend(b"fmt ");
        out.extend((fmt.len() as u32).to_le_bytes());
        out.extend(&fmt);
        // An odd-sized chunk to skip, with its pad byte.
        out.extend(b"LIST");
        out.extend(3u32.to_le_bytes());
        out.extend([1, 2, 3, 0]);
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        let riff = (out.len() - 8) as u32;
        out[4..8].copy_from_slice(&riff.to_le_bytes());
        out
    }

    #[test]
    fn pcm16_passes_through() {
        let samples: Vec<u8> = (0..400u16).flat_map(|s| s.to_le_bytes()).collect();
        let c = parse_wav(&wav(PCM, 2, 44_100, 16, &samples)).unwrap();
        assert_eq!(c.format, SampleFormat::S16);
        assert_eq!((c.rate, c.channels), (44_100, 2));
        assert_eq!(c.data, samples);
        assert_eq!(c.frames(), 200);
        assert_eq!(c.converted_from, None);
    }

    #[test]
    fn pcm8_passes_through() {
        let c = parse_wav(&wav(PCM, 1, 8_000, 8, &[128; 8_000])).unwrap();
        assert_eq!(c.format, SampleFormat::U8);
        assert_eq!(c.duration(), Duration::from_secs(1));
    }

    #[test]
    fn wide_and_float_samples_become_16_bit() {
        // 24-bit: 0x123456 → 0x1234.
        let c = parse_wav(&wav(
            PCM,
            1,
            16_000,
            24,
            &[0x56, 0x34, 0x12, 0xff, 0xff, 0x7f],
        ))
        .unwrap();
        assert_eq!(c.data, vec![0x34, 0x12, 0xff, 0x7f]);
        assert_eq!(c.converted_from, Some("pcm24"));
        // 32-bit: the top two bytes.
        let c = parse_wav(&wav(PCM, 1, 16_000, 32, &[1, 2, 3, 4])).unwrap();
        assert_eq!(c.data, vec![3, 4]);
        // Float: full scale, clipped, silence.
        let f: Vec<u8> = [1.0f32, -2.0, 0.0, f32::NAN]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let c = parse_wav(&wav(FLOAT, 2, 48_000, 32, &f)).unwrap();
        let s: Vec<i16> = c
            .data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect();
        assert_eq!(s, vec![32767, -32767, 0, 0]);
        assert_eq!(c.converted_from, Some("float32"));
        let f: Vec<u8> = [0.5f64].iter().flat_map(|v| v.to_le_bytes()).collect();
        let c = parse_wav(&wav(FLOAT, 1, 48_000, 64, &f)).unwrap();
        assert_eq!(c.data, 16384i16.to_le_bytes().to_vec());
    }

    #[test]
    fn extensible_uses_the_subformat() {
        let mut fmt = Vec::new();
        fmt.extend(EXTENSIBLE.to_le_bytes());
        fmt.extend(1u16.to_le_bytes());
        fmt.extend(22_050u32.to_le_bytes());
        fmt.extend(44_100u32.to_le_bytes());
        fmt.extend(2u16.to_le_bytes());
        fmt.extend(16u16.to_le_bytes());
        fmt.extend(22u16.to_le_bytes()); // cbSize
        fmt.extend(16u16.to_le_bytes()); // valid bits
        fmt.extend(4u32.to_le_bytes()); // channel mask
        fmt.extend(PCM.to_le_bytes());
        fmt.extend([0u8; 14]);
        let mut b = Vec::new();
        b.extend(b"RIFF\0\0\0\0WAVEfmt ");
        b.extend((fmt.len() as u32).to_le_bytes());
        b.extend(fmt);
        b.extend(b"data");
        b.extend(4u32.to_le_bytes());
        b.extend([1, 0, 2, 0]);
        let c = parse_wav(&b).unwrap();
        assert_eq!(
            (c.rate, c.format, c.frames()),
            (22_050, SampleFormat::S16, 2)
        );
    }

    #[test]
    fn unknown_data_size_runs_to_the_end_and_partial_frames_are_dropped() {
        let mut b = wav(PCM, 2, 44_100, 16, &[0; 9]);
        let at = b.len() - 9 - 4;
        b[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let c = parse_wav(&b).unwrap();
        assert_eq!(c.data.len(), 8);
        // Size 0 (a stream that was never finalised) also reads to the end.
        let at = b.len() - 9 - 4;
        b[at..at + 4].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(parse_wav(&b).unwrap().data.len(), 8);
    }

    #[test]
    fn unsupported_files_are_usage_errors() {
        let usage = |r: JidrResult<Clip>| {
            let e = r.unwrap_err();
            assert_eq!(e.code, jidr_core::ErrorCode::Usage, "{}", e.message);
            e.message
        };
        assert!(usage(parse_wav(b"ID3\x04 not a wav")).contains("not a WAV"));
        assert!(usage(parse_wav(&wav(PCM, 1, 96_000, 16, &[0; 4]))).contains("96000 Hz"));
        assert!(usage(parse_wav(&wav(PCM, 1, 4_000, 16, &[0; 4]))).contains("4000 Hz"));
        assert!(usage(parse_wav(&wav(PCM, 6, 48_000, 16, &[0; 12]))).contains("6 channels"));
        assert!(usage(parse_wav(&wav(0x55, 2, 44_100, 0, &[0; 4]))).contains("MP3"));
        assert!(usage(parse_wav(&wav(7, 1, 8_000, 8, &[0; 4]))).contains("µ-law"));
        assert!(usage(parse_wav(&wav(PCM, 1, 8_000, 12, &[0; 4]))).contains("12-bit PCM"));
        assert!(usage(parse_wav(&wav(PCM, 1, 8_000, 16, &[]))).contains("no samples"));
        let mut no_data = wav(PCM, 1, 8_000, 16, &[]);
        no_data.truncate(no_data.len() - 8);
        assert!(usage(parse_wav(&no_data)).contains("no data chunk"));
        let mut b = Vec::new();
        b.extend(b"RIFF\0\0\0\0WAVEdata");
        b.extend(2u32.to_le_bytes());
        b.extend([0, 0]);
        assert!(usage(parse_wav(&b)).contains("no fmt chunk"));
    }

    #[test]
    fn raw_pcm_needs_a_valid_layout() {
        let c = raw_s16(vec![0; 3201], 16_000, 1).unwrap();
        assert_eq!(c.data.len(), 3200);
        assert_eq!(c.duration(), Duration::from_millis(100));
        assert!(raw_s16(vec![0; 4], 192_000, 1).is_err());
        assert!(raw_s16(vec![0; 4], 16_000, 0).is_err());
    }

    #[test]
    fn load_tells_wav_and_raw_apart() {
        let dir = tempfile::tempdir().unwrap();
        let w = dir.path().join("a.wav");
        std::fs::write(&w, wav(PCM, 1, 16_000, 16, &[0; 320])).unwrap();
        assert_eq!(load(&w, None).unwrap().frames(), 160);
        assert!(
            load(&w, Some((16_000, 1)))
                .unwrap_err()
                .message
                .contains("is a WAV file")
        );
        let r = dir.path().join("a.pcm");
        std::fs::write(&r, [0u8; 640]).unwrap();
        assert!(
            load(&r, None)
                .unwrap_err()
                .message
                .contains("isn't a WAV file")
        );
        assert_eq!(load(&r, Some((32_000, 2))).unwrap().frames(), 160);
        let e = load(&dir.path().join("missing.wav"), None).unwrap_err();
        assert_eq!(e.code, jidr_core::ErrorCode::Usage);
    }

    #[test]
    fn packets_hold_whole_frames() {
        let c = raw_s16(vec![0; 4], 44_100, 2).unwrap();
        assert_eq!(c.packet_bytes(20), 882 * 4);
        let c = raw_s16(vec![0; 4], 11_025, 1).unwrap();
        assert_eq!(c.packet_bytes(20), 220 * 2);
        assert_eq!(c.packet_bytes(0), 2);
    }

    #[test]
    fn durations_are_exact() {
        assert_eq!(frames_duration(44_100, 44_100), Duration::from_secs(1));
        assert_eq!(frames_duration(22_050, 44_100), Duration::from_millis(500));
        assert_eq!(frames_duration(5, 0), Duration::ZERO);
    }
}
