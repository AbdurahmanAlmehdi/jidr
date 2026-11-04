//! Microphone injection (#70): streams a clip to the emulator's
//! `EmulatorController.injectAudio(stream AudioPacket)`.
//!
//! Per the proto: only the first packet's `format` is honoured, so it is sent
//! once; the emulator keeps a buffer of about 300 ms and, in the default
//! delivery mode (`MODE_UNSPECIFIED`), blocks the stream until the guest
//! reads its microphone. Jidr uses that mode (`MODE_REAL_TIME` is marked
//! experimental and overwrites the buffer) and also paces packets to real
//! time, at most [`LEAD`] ahead, so progress tracks what the device heard
//! and a stop takes effect within a packet or two. `FAILED_PRECONDITION`
//! means another client already feeds the microphone.

pub mod wav;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use jidr_core::{Cancel, JidrError, JidrResult};
use jidr_proto::Controller;
use jidr_proto::control as pb;
use tokio::sync::mpsc;
use tokio::time::Instant;

pub use wav::{Clip, SampleFormat};

/// Audio per packet.
pub const PACKET_MS: u32 = 20;
/// How far ahead of real time packets are sent (inside the emulator's
/// ~300 ms buffer).
pub const LEAD: Duration = Duration::from_millis(100);
/// Behind schedule by more than this (the stream was blocked because nothing
/// read the microphone): restart the schedule instead of bursting.
pub const RESYNC: Duration = Duration::from_millis(500);
/// How often a blocked stream checks for a stop or cancel.
const POLL: Duration = Duration::from_millis(100);
/// How long to wait for the emulator's answer once the stream has ended.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

/// The proto `AudioFormat` for a clip (input: blocking delivery).
// The build may use the installed SDK's newer protos, which can add fields.
#[allow(clippy::needless_update)]
pub fn format_pb(clip: &Clip) -> pb::AudioFormat {
    pb::AudioFormat {
        sampling_rate: u64::from(clip.rate),
        channels: if clip.channels == 2 {
            pb::audio_format::Channels::Stereo as i32
        } else {
            pb::audio_format::Channels::Mono as i32
        },
        format: match clip.format {
            SampleFormat::U8 => pb::audio_format::SampleFormat::AudFmtU8 as i32,
            SampleFormat::S16 => pb::audio_format::SampleFormat::AudFmtS16 as i32,
        },
        mode: pb::audio_format::DeliveryMode::ModeUnspecified as i32,
        ..Default::default()
    }
}

/// How to play a clip.
#[derive(Debug, Clone, Copy)]
pub struct Playback {
    /// Start again at the end, until stopped.
    pub repeat: bool,
    pub packet_ms: u32,
    pub lead: Duration,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            repeat: false,
            packet_ms: PACKET_MS,
            lead: LEAD,
        }
    }
}

/// What was played.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Played {
    /// Audio sent, across loops.
    pub played: Duration,
    /// Full passes through the clip.
    pub loops: u32,
    /// Ended by a stop request rather than the end of the clip.
    pub stopped: bool,
}

/// Before sending audio that starts `sent` into the stream: how long to wait,
/// and the (possibly moved) start of the schedule.
pub fn pace(start: Instant, sent: Duration, lead: Duration, now: Instant) -> (Duration, Instant) {
    let due = (start + sent).checked_sub(lead).unwrap_or(start);
    if now < due {
        (due - now, start)
    } else if now - due > RESYNC {
        // Blocked for a while: carry on from here at real time.
        (
            Duration::ZERO,
            (now + lead).checked_sub(sent).unwrap_or(now),
        )
    } else {
        (Duration::ZERO, start)
    }
}

/// Sends the clip's packets into `tx` (the request stream of
/// `injectAudio`), paced to real time, until the end (or forever with
/// `repeat`), a `stop`, a cancel, or the receiver closing (the call
/// ended). `progress` gets the audio sent so far and the finished loops,
/// once per packet.
// The build may use the installed SDK's newer protos, which can add fields.
#[allow(clippy::needless_update)]
pub async fn feed(
    tx: mpsc::Sender<pb::AudioPacket>,
    clip: &Clip,
    play: Playback,
    stop: &AtomicBool,
    cancel: &Cancel,
    progress: &(dyn Fn(Duration, u32) + Send + Sync),
) -> JidrResult<Played> {
    let chunk = clip.packet_bytes(play.packet_ms).max(clip.frame_bytes());
    let frame = clip.frame_bytes().max(1) as u64;
    let mut start = Instant::now();
    let mut frames = 0u64;
    let mut loops = 0u32;
    let mut first = true;
    loop {
        for bytes in clip.data.chunks(chunk) {
            let sent = wav::frames_duration(frames, clip.rate);
            let (wait, s) = pace(start, sent, play.lead, Instant::now());
            start = s;
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
            let packet = pb::AudioPacket {
                format: first.then(|| format_pb(clip)),
                timestamp: u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_micros(),
                )
                .unwrap_or(0),
                audio: bytes.to_vec(),
                ..Default::default()
            };
            // Wait for room in the stream, checking for a stop meanwhile.
            let permit = loop {
                cancel.check()?;
                if stop.load(Ordering::Relaxed) {
                    return Ok(Played {
                        played: wav::frames_duration(frames, clip.rate),
                        loops,
                        stopped: true,
                    });
                }
                tokio::select! {
                    p = tx.reserve() => break p,
                    () = tokio::time::sleep(POLL) => {}
                }
            };
            let Ok(permit) = permit else {
                // The call ended (its error is the caller's to report).
                return Ok(Played {
                    played: wav::frames_duration(frames, clip.rate),
                    loops,
                    stopped: false,
                });
            };
            permit.send(packet);
            first = false;
            frames += bytes.len() as u64 / frame;
            progress(wav::frames_duration(frames, clip.rate), loops);
        }
        loops += 1;
        if !play.repeat {
            break;
        }
    }
    // Let the last packets play out before the stream closes.
    let end = start + wav::frames_duration(frames, clip.rate);
    while Instant::now() < end {
        cancel.check()?;
        if stop.load(Ordering::Relaxed) {
            break;
        }
        tokio::time::sleep(POLL.min(end.saturating_duration_since(Instant::now()))).await;
    }
    Ok(Played {
        played: wav::frames_duration(frames, clip.rate),
        loops,
        stopped: stop.load(Ordering::Relaxed),
    })
}

/// Maps an `injectAudio` failure, naming the usual causes.
pub fn inject_error(s: tonic::Status) -> JidrError {
    match s.code() {
        tonic::Code::FailedPrecondition => JidrError::busy(format!(
            "another client is already feeding this device's microphone ({})",
            s.message()
        ))
        .with_data(serde_json::json!({ "grpc_code": "FailedPrecondition" })),
        tonic::Code::Unimplemented => JidrError::sdk(
            "this emulator doesn't support audio injection (injectAudio); update the Android Emulator",
        )
        .with_data(serde_json::json!({ "grpc_code": "Unimplemented" })),
        _ => crate::grpc::status_error("injectAudio", s),
    }
}

/// Plays `clip` as the device's microphone over `injectAudio`.
pub async fn inject(
    mut controller: Controller,
    clip: &Clip,
    play: Playback,
    stop: &AtomicBool,
    cancel: &Cancel,
    progress: &(dyn Fn(Duration, u32) + Send + Sync),
) -> JidrResult<Played> {
    let (tx, rx) = mpsc::channel(4);
    let mut call = tokio::spawn(async move {
        controller
            .inject_audio(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
    });
    // Returns when the clip ends, on a stop or cancel, or when the call ends
    // early; the stream closes with it.
    let fed = feed(tx, clip, play, stop, cancel, progress).await;
    let answer = tokio::time::timeout(ANSWER_TIMEOUT, &mut call).await;
    call.abort();
    let played = fed?;
    match answer {
        // A refused call (busy microphone, bad rate) closes the stream
        // early; its status says why.
        Ok(Ok(Err(status))) => Err(inject_error(status)),
        Ok(Ok(Ok(_))) => Ok(played),
        Ok(Err(e)) => Err(JidrError::unexpected(format!("injectAudio task: {e}"))),
        Err(_) => {
            tracing::warn!("injectAudio didn't answer after the stream ended");
            Ok(played)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn clip(rate: u32, channels: u16, ms_len: u64) -> Clip {
        let frames = u64::from(rate) * ms_len / 1000;
        let data = (0..frames * u64::from(channels))
            .flat_map(|i| (i as i16).to_le_bytes())
            .collect();
        wav::raw_s16(data, rate, channels).unwrap()
    }

    async fn collect(
        clip: &Clip,
        play: Playback,
        stop: Arc<AtomicBool>,
        cancel: Cancel,
        stop_after: Option<usize>,
    ) -> (JidrResult<Played>, Vec<pb::AudioPacket>) {
        let (tx, mut rx) = mpsc::channel(4);
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        let s = stop.clone();
        let reader = tokio::spawn(async move {
            while let Some(p) = rx.recv().await {
                let mut v = g.lock().unwrap();
                v.push(p);
                if stop_after.is_some_and(|n| v.len() >= n) {
                    s.store(true, Ordering::Relaxed);
                }
            }
        });
        let r = feed(tx, clip, play, &stop, &cancel, &|_, _| {}).await;
        reader.await.unwrap();
        let v = std::mem::take(&mut *got.lock().unwrap());
        (r, v)
    }

    #[test]
    fn format_maps_to_the_proto() {
        let f = format_pb(&clip(44_100, 2, 10));
        assert_eq!(f.sampling_rate, 44_100);
        assert_eq!(f.channels, pb::audio_format::Channels::Stereo as i32);
        assert_eq!(f.format, pb::audio_format::SampleFormat::AudFmtS16 as i32);
        assert_eq!(
            f.mode,
            pb::audio_format::DeliveryMode::ModeUnspecified as i32
        );
        let u8clip = wav::parse_wav(&wav::tests::wav(1, 1, 8_000, 8, &[128; 16])).unwrap();
        let f = format_pb(&u8clip);
        assert_eq!(f.channels, pb::audio_format::Channels::Mono as i32);
        assert_eq!(f.format, pb::audio_format::SampleFormat::AudFmtU8 as i32);
    }

    #[test]
    fn pacing_stays_a_lead_ahead_and_resyncs_after_a_block() {
        let t0 = Instant::now();
        // At the start: the first 100 ms go at once.
        assert_eq!(pace(t0, ms(0), ms(100), t0), (Duration::ZERO, t0));
        assert_eq!(pace(t0, ms(100), ms(100), t0), (Duration::ZERO, t0));
        // 300 ms in, at t0: wait 200 ms.
        assert_eq!(pace(t0, ms(300), ms(100), t0), (ms(200), t0));
        // Slightly late: send now, keep the schedule.
        assert_eq!(
            pace(t0, ms(300), ms(100), t0 + ms(400)),
            (Duration::ZERO, t0)
        );
        // Blocked for seconds: send now and move the schedule.
        let now = t0 + ms(5_000);
        let (wait, start) = pace(t0, ms(300), ms(100), now);
        assert_eq!(wait, Duration::ZERO);
        assert_eq!(start + ms(300) - ms(100), now);
    }

    #[tokio::test]
    async fn packets_cover_the_clip_with_the_format_once() {
        let c = clip(16_000, 1, 130);
        let play = Playback {
            lead: ms(1_000),
            ..Playback::default()
        };
        let (r, packets) = collect(&c, play, Arc::default(), Cancel::new(), None).await;
        let r = r.unwrap();
        assert_eq!((r.loops, r.stopped), (1, false));
        assert_eq!(r.played, ms(130));
        // 20 ms of 16 kHz mono = 640 bytes; 130 ms = 6 full packets + 10 ms.
        let sizes: Vec<usize> = packets.iter().map(|p| p.audio.len()).collect();
        assert_eq!(sizes, vec![640, 640, 640, 640, 640, 640, 320]);
        assert!(packets[0].format.is_some());
        assert!(packets[1..].iter().all(|p| p.format.is_none()));
        let joined: Vec<u8> = packets.iter().flat_map(|p| p.audio.clone()).collect();
        assert_eq!(joined, c.data);
        assert!(packets[0].timestamp > 0);
    }

    #[tokio::test]
    async fn packets_are_paced_to_real_time() {
        let c = clip(8_000, 1, 300);
        let play = Playback {
            lead: ms(50),
            ..Playback::default()
        };
        let t0 = std::time::Instant::now();
        let (r, packets) = collect(&c, play, Arc::default(), Cancel::new(), None).await;
        r.unwrap();
        assert_eq!(packets.len(), 15);
        // 300 ms of audio, sent no more than 50 ms ahead, then played out.
        assert!(t0.elapsed() >= ms(290), "{:?}", t0.elapsed());
    }

    #[tokio::test]
    async fn stop_ends_the_stream() {
        let c = clip(8_000, 1, 10_000);
        let stop = Arc::new(AtomicBool::new(false));
        let play = Playback {
            repeat: true,
            lead: ms(60_000),
            ..Playback::default()
        };
        let (r, packets) = collect(&c, play, stop, Cancel::new(), Some(10)).await;
        let r = r.unwrap();
        assert!(r.stopped);
        assert!(
            packets.len() >= 10 && packets.len() < 20,
            "{}",
            packets.len()
        );
        assert_eq!(r.played, ms(20) * packets.len() as u32);
    }

    #[tokio::test]
    async fn repeat_loops_until_stopped() {
        let c = clip(8_000, 1, 40);
        let play = Playback {
            repeat: true,
            lead: ms(60_000),
            ..Playback::default()
        };
        // 2 packets per pass: stop after 5 passes.
        let (r, packets) = collect(&c, play, Arc::default(), Cancel::new(), Some(10)).await;
        let r = r.unwrap();
        assert!(r.stopped);
        assert!(r.loops >= 5, "{}", r.loops);
        assert_eq!(packets.iter().filter(|p| p.format.is_some()).count(), 1);
    }

    #[tokio::test]
    async fn cancel_is_an_error_and_a_closed_stream_is_not() {
        let c = clip(8_000, 1, 1_000);
        let cancel = Cancel::new();
        cancel.cancel();
        let (r, _) = collect(&c, Playback::default(), Arc::default(), cancel, None).await;
        assert!(jidr_core::cancel::is_cancelled(&r.unwrap_err()));

        let (tx, rx) = mpsc::channel(4);
        drop(rx);
        let r = feed(
            tx,
            &c,
            Playback::default(),
            &AtomicBool::new(false),
            &Cancel::new(),
            &|_, _| {},
        )
        .await
        .unwrap();
        assert_eq!((r.played, r.stopped), (Duration::ZERO, false));
    }

    #[tokio::test]
    async fn a_blocked_stream_still_stops() {
        // Nobody reads: the channel fills and the feeder must still see the stop.
        let c = clip(8_000, 1, 1_000);
        let (tx, _rx) = mpsc::channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        tokio::spawn(async move {
            tokio::time::sleep(ms(300)).await;
            s.store(true, Ordering::Relaxed);
        });
        let play = Playback {
            lead: ms(60_000),
            ..Playback::default()
        };
        let r = tokio::time::timeout(
            ms(5_000),
            feed(tx, &c, play, &stop, &Cancel::new(), &|_, _| {}),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(r.stopped);
        assert_eq!(r.played, ms(20));
    }

    #[test]
    fn grpc_errors_name_the_cause() {
        let e = inject_error(tonic::Status::failed_precondition("mic registered"));
        assert_eq!(e.code, jidr_core::ErrorCode::Busy);
        let e = inject_error(tonic::Status::unimplemented(""));
        assert_eq!(e.code, jidr_core::ErrorCode::Sdk);
        let e = inject_error(tonic::Status::invalid_argument("rate too high"));
        assert_eq!(e.code, jidr_core::ErrorCode::Usage);
        assert!(e.message.contains("rate too high"));
    }
}
