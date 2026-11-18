use serde::{Deserialize, Serialize};

/// Every knob of the image + scrolling-spectrum render.
/// `Default` reproduces the original hand-written ffmpeg command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // Files
    pub image: String,
    pub audio: String,
    pub output: String,
    // Video
    pub width: i64,
    pub height: i64,
    pub fps: i64,
    pub fit: String,
    // Spectrum
    pub orientation: String,
    pub mode: String,
    pub slide: String,
    pub color: String,
    pub scale: String,
    pub fscale: String,
    pub legend: bool,
    pub vflip: bool,
    // Blend
    pub blend_mode: String,
    pub opacity: f64,
    // x264
    pub preset: String,
    pub crf: i64,
    pub profile: String,
    pub bframes: i64,
    pub gop: i64,
    pub threads: i64,
    pub bt709: bool,
    // Audio
    pub audio_codec: String,
    pub audio_bitrate: i64,
    pub sample_rate: String,
    // Process
    pub nice: i64,
    pub shortest: bool,
    pub faststart: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            image: String::new(),
            audio: String::new(),
            output: "out.mp4".into(),
            width: 1920,
            height: 1080,
            fps: 30,
            fit: "fill".into(),
            orientation: "horizontal".into(),
            mode: "combined".into(),
            slide: "scroll".into(),
            color: "green".into(),
            scale: "log".into(),
            fscale: "log".into(),
            legend: false,
            vflip: true,
            blend_mode: "screen".into(),
            opacity: 0.85,
            preset: "veryslow".into(),
            crf: 16,
            profile: "high".into(),
            bframes: 2,
            gop: 15,
            threads: 2,
            bt709: true,
            audio_codec: "aac".into(),
            audio_bitrate: 384,
            sample_rate: "48000".into(),
            nice: 19,
            shortest: true,
            faststart: true,
        }
    }
}
