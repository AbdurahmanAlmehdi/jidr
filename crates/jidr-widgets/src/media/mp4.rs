//! MP4/H.264 writer on AVFoundation's `AVAssetWriter`, which encodes with
//! VideoToolbox in hardware (D13). Frames come in as RGBA, are copied into
//! BGRA pixel buffers from the writer's pool and stamped at a fixed rate.
//!
//! `unsafe` is allowed here only: these are FFI calls into Apple frameworks
//! through the `objc2` bindings. Every call keeps to the documented API:
//! objects are created and used on one thread, pixel buffers are locked
//! while written, and the writer is finished before it's dropped.
#![allow(unsafe_code)]

use std::path::Path;
use std::ptr::NonNull;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use jidr_core::{JidrError, JidrResult};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_av_foundation::{
    AVAssetWriter, AVAssetWriterInput, AVAssetWriterInputPixelBufferAdaptor, AVAssetWriterStatus,
    AVFileTypeMPEG4, AVMediaTypeVideo, AVVideoAverageBitRateKey, AVVideoCodecKey,
    AVVideoCodecTypeH264, AVVideoCompressionPropertiesKey, AVVideoHeightKey, AVVideoWidthKey,
};
use objc2_core_foundation::{CFRetained, CFString};
use objc2_core_media::{CMTime, CMTimeFlags};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferPool,
    CVPixelBufferUnlockBaseAddress, kCVPixelBufferHeightKey, kCVPixelBufferPixelFormatTypeKey,
    kCVPixelBufferWidthKey, kCVPixelFormatType_32BGRA, kCVReturnSuccess,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};

fn err(what: impl Into<String>) -> JidrError {
    JidrError::unexpected(format!("recording: {}", what.into()))
}

/// CF strings are toll-free bridged to `NSString`.
fn ns(key: &CFString) -> &NSString {
    // SAFETY: CFString and NSString are toll-free bridged; same object.
    unsafe { &*(std::ptr::from_ref(key).cast::<NSString>()) }
}

fn time(frame: i64, fps: i32) -> CMTime {
    CMTime {
        value: frame,
        timescale: fps,
        flags: CMTimeFlags::Valid,
        epoch: 0,
    }
}

pub struct Mp4Writer {
    writer: Retained<AVAssetWriter>,
    input: Retained<AVAssetWriterInput>,
    adaptor: Retained<AVAssetWriterInputPixelBufferAdaptor>,
    width: u32,
    height: u32,
    fps: i32,
    frames: i64,
}

impl Mp4Writer {
    /// Starts an H.264 MP4 at `path` (must not exist). Sides are rounded
    /// down to even numbers, as H.264 requires.
    pub fn create(path: &Path, width: u32, height: u32, fps: i32) -> JidrResult<Self> {
        let (width, height) = (width & !1, height & !1);
        if width < 16 || height < 16 {
            return Err(err("frame too small"));
        }
        let path_str = path.to_str().ok_or_else(|| err("path isn't UTF-8"))?;
        // SAFETY: documented AVFoundation calls with valid, retained arguments.
        unsafe {
            let url = NSURL::fileURLWithPath(&NSString::from_str(path_str));
            let file_type = AVFileTypeMPEG4.ok_or_else(|| err("no MPEG-4 file type"))?;
            let writer = AVAssetWriter::assetWriterWithURL_fileType_error(&url, file_type)
                .map_err(|e| err(e.localizedDescription().to_string()))?;

            let bitrate = NSNumber::new_u32((width * height * 4).clamp(2_000_000, 16_000_000));
            let compression = NSDictionary::<NSString, AnyObject>::from_slices(
                &[AVVideoAverageBitRateKey.ok_or_else(|| err("no bitrate key"))?],
                &[&*bitrate as &AnyObject],
            );
            let w = NSNumber::new_u32(width);
            let h = NSNumber::new_u32(height);
            let codec = AVVideoCodecTypeH264.ok_or_else(|| err("no H.264 codec"))?;
            let settings = NSDictionary::<NSString, AnyObject>::from_slices(
                &[
                    AVVideoCodecKey.ok_or_else(|| err("no codec key"))?,
                    AVVideoWidthKey.ok_or_else(|| err("no width key"))?,
                    AVVideoHeightKey.ok_or_else(|| err("no height key"))?,
                    AVVideoCompressionPropertiesKey.ok_or_else(|| err("no compression key"))?,
                ],
                &[
                    &**codec as &AnyObject,
                    &*w as &AnyObject,
                    &*h as &AnyObject,
                    &*compression as &AnyObject,
                ],
            );
            let media = AVMediaTypeVideo.ok_or_else(|| err("no video media type"))?;
            let input = AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
                media,
                Some(&settings),
            );
            input.setExpectsMediaDataInRealTime(true);

            let fmt = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
            let attrs = NSDictionary::<NSString, AnyObject>::from_slices(
                &[
                    ns(kCVPixelBufferPixelFormatTypeKey),
                    ns(kCVPixelBufferWidthKey),
                    ns(kCVPixelBufferHeightKey),
                ],
                &[&*fmt as &AnyObject, &*w as &AnyObject, &*h as &AnyObject],
            );
            let adaptor = AVAssetWriterInputPixelBufferAdaptor::assetWriterInputPixelBufferAdaptorWithAssetWriterInput_sourcePixelBufferAttributes(
                &input,
                Some(&attrs),
            );
            if !writer.canAddInput(&input) {
                return Err(err("the writer refused the video input"));
            }
            writer.addInput(&input);
            if !writer.startWriting() {
                return Err(err(writer
                    .error()
                    .map(|e| e.localizedDescription().to_string())
                    .unwrap_or_else(|| "can't start writing".into())));
            }
            writer.startSessionAtSourceTime(time(0, fps));
            Ok(Self {
                writer,
                input,
                adaptor,
                width,
                height,
                fps,
                frames: 0,
            })
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn frames(&self) -> i64 {
        self.frames
    }

    fn pool(&self) -> JidrResult<Retained<CVPixelBufferPool>> {
        // SAFETY: the pool exists once writing has started.
        unsafe { self.adaptor.pixelBufferPool() }.ok_or_else(|| err("no pixel buffer pool"))
    }

    /// Appends one RGBA frame of exactly `size()` as the next frame.
    pub fn push(&mut self, rgba: &[u8]) -> JidrResult<()> {
        let (w, h) = (self.width as usize, self.height as usize);
        if rgba.len() < w * h * 4 {
            return Err(err("short frame"));
        }
        // Wait (briefly) for the encoder to take more data.
        let deadline = Instant::now() + Duration::from_millis(500);
        // SAFETY: plain property read on the input we own.
        while !unsafe { self.input.isReadyForMoreMediaData() } {
            if Instant::now() > deadline {
                return Err(err("the encoder stopped taking frames"));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let pool = self.pool()?;
        let mut out: *mut CVPixelBuffer = std::ptr::null_mut();
        // SAFETY: valid pool and out-pointer; on success we own the buffer (+1).
        let rc =
            unsafe { CVPixelBufferPool::create_pixel_buffer(None, &pool, NonNull::from(&mut out)) };
        if rc != kCVReturnSuccess {
            return Err(err(format!("no pixel buffer ({rc})")));
        }
        let buf: CFRetained<CVPixelBuffer> = unsafe {
            // SAFETY: created above with a +1 retain count.
            CFRetained::from_raw(NonNull::new(out).ok_or_else(|| err("null pixel buffer"))?)
        };
        // SAFETY: lock, write within bytes_per_row × height, unlock.
        unsafe {
            if CVPixelBufferLockBaseAddress(&buf, CVPixelBufferLockFlags(0)) != kCVReturnSuccess {
                return Err(err("can't lock the pixel buffer"));
            }
            let base = CVPixelBufferGetBaseAddress(&buf).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRow(&buf);
            if base.is_null() || stride < w * 4 {
                CVPixelBufferUnlockBaseAddress(&buf, CVPixelBufferLockFlags(0));
                return Err(err("bad pixel buffer"));
            }
            for y in 0..h {
                let dst = std::slice::from_raw_parts_mut(base.add(y * stride), w * 4);
                let src = &rgba[y * w * 4..(y + 1) * w * 4];
                for (d, s) in dst
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(src.as_chunks::<4>().0)
                {
                    *d = [s[2], s[1], s[0], 255];
                }
            }
            CVPixelBufferUnlockBaseAddress(&buf, CVPixelBufferLockFlags(0));
        }
        // SAFETY: append from the writer's own thread with a valid buffer.
        let ok = unsafe {
            self.adaptor
                .appendPixelBuffer_withPresentationTime(&buf, time(self.frames, self.fps))
        };
        if !ok {
            return Err(err(self.failure()));
        }
        self.frames += 1;
        Ok(())
    }

    fn failure(&self) -> String {
        // SAFETY: property reads.
        unsafe {
            self.writer
                .error()
                .map(|e| e.localizedDescription().to_string())
                .unwrap_or_else(|| format!("writer status {:?}", self.writer.status()))
        }
    }

    /// Finishes the file and waits for it to be written.
    pub fn finish(self) -> JidrResult<()> {
        let (tx, rx) = mpsc::channel::<()>();
        let block = block2::RcBlock::new(move || {
            let _ = tx.send(());
        });
        // SAFETY: all appends returned; the block only signals a channel.
        unsafe {
            self.input.markAsFinished();
            self.writer
                .endSessionAtSourceTime(time(self.frames.max(1), self.fps));
            self.writer.finishWritingWithCompletionHandler(&block);
        }
        rx.recv_timeout(Duration::from_secs(60))
            .map_err(|_| err("finishing the file timed out"))?;
        // SAFETY: property read.
        if unsafe { self.writer.status() } == AVAssetWriterStatus::Completed {
            Ok(())
        } else {
            Err(err(self.failure()))
        }
    }

    /// Abandons the file.
    pub fn cancel(self) {
        // SAFETY: documented cancel.
        unsafe { self.writer.cancelWriting() };
    }
}
