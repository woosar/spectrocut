use super::settings::Settings;
use crate::helpers::fmt_float;

#[derive(Clone, Copy, PartialEq)]
pub enum PathKind {
    Image,
    Audio,
}

impl PathKind {
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            PathKind::Image => &["jpg", "jpeg", "png", "webp", "bmp", "gif", "tif", "tiff"],
            PathKind::Audio => &[
                "wav", "flac", "mp3", "ogg", "opus", "m4a", "aac", "aif", "aiff", "wma",
            ],
        }
    }
}

pub enum Kind {
    Path(PathKind),
    Text,
    Int { min: i64, max: i64, step: i64 },
    Float { min: f64, max: f64, step: f64 },
    Choice(&'static [&'static str]),
    Toggle,
}

/// Typed access to one member of `Settings`.
enum Access {
    Str(fn(&Settings) -> &String, fn(&mut Settings) -> &mut String),
    Int(fn(&Settings) -> &i64, fn(&mut Settings) -> &mut i64),
    Float(fn(&Settings) -> &f64, fn(&mut Settings) -> &mut f64),
    Bool(fn(&Settings) -> &bool, fn(&mut Settings) -> &mut bool),
}

/// One editable row of the settings form.
pub struct Field {
    pub group: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: Kind,
    access: Access,
}

macro_rules! field {
    ($group:expr, $label:expr, $help:expr, $kind:expr, $variant:ident, $member:ident) => {
        Field {
            group: $group,
            label: $label,
            help: $help,
            kind: $kind,
            access: Access::$variant(|s: &Settings| &s.$member, |s: &mut Settings| &mut s.$member),
        }
    };
}

const COLORS: &[&str] = &[
    "channel",
    "intensity",
    "rainbow",
    "moreland",
    "nebulae",
    "fire",
    "fiery",
    "fruit",
    "cool",
    "magma",
    "green",
    "viridis",
    "plasma",
    "cividis",
    "terrain",
];
const BLEND_MODES: &[&str] = &[
    "screen",
    "addition",
    "lighten",
    "darken",
    "overlay",
    "softlight",
    "hardlight",
    "multiply",
    "difference",
    "exclusion",
    "dodge",
    "burn",
    "average",
    "normal",
];
const X264_PRESETS: &[&str] = &[
    "ultrafast",
    "superfast",
    "veryfast",
    "faster",
    "fast",
    "medium",
    "slow",
    "slower",
    "veryslow",
    "placebo",
];

impl Field {
    pub fn all() -> Vec<Field> {
        use Kind::*;
        vec![
            field!(
                "Files",
                "Image",
                "Background still image (Enter opens file browser)",
                Path(PathKind::Image),
                Str,
                image
            ),
            field!(
                "Files",
                "Audio",
                "Audio track: drives the spectrum and is the soundtrack",
                Path(PathKind::Audio),
                Str,
                audio
            ),
            field!(
                "Files",
                "Output",
                "Output .mp4 file (Enter to type a name)",
                Text,
                Str,
                output
            ),
            field!(
                "Video",
                "Width",
                "Output width in pixels (must be even)",
                Int {
                    min: 16,
                    max: 7680,
                    step: 16
                },
                Int,
                width
            ),
            field!(
                "Video",
                "Height",
                "Output height in pixels (must be even)",
                Int {
                    min: 16,
                    max: 4320,
                    step: 16
                },
                Int,
                height
            ),
            field!(
                "Video",
                "FPS",
                "Frame rate of video and spectrum",
                Int {
                    min: 1,
                    max: 120,
                    step: 1
                },
                Int,
                fps
            ),
            field!(
                "Video",
                "Image fit",
                "fill = scale up and crop, fit = letterbox with black bars",
                Choice(&["fill", "fit"]),
                Str,
                fit
            ),
            field!(
                "Spectrum",
                "Orientation",
                "Direction the spectrum runs",
                Choice(&["horizontal", "vertical"]),
                Str,
                orientation
            ),
            field!(
                "Spectrum",
                "Channels",
                "combined = one display, separate = one per channel",
                Choice(&["combined", "separate"]),
                Str,
                mode
            ),
            field!(
                "Spectrum",
                "Slide",
                "How new columns enter the picture",
                Choice(&["scroll", "rscroll", "replace", "lreplace", "fullframe"]),
                Str,
                slide
            ),
            field!(
                "Spectrum",
                "Color",
                "Colour scheme of the spectrum",
                Choice(COLORS),
                Str,
                color
            ),
            field!(
                "Spectrum",
                "Intensity scale",
                "Amplitude to brightness mapping",
                Choice(&["log", "lin", "sqrt", "cbrt", "4thrt", "5thrt"]),
                Str,
                scale
            ),
            field!(
                "Spectrum",
                "Freq scale",
                "Frequency axis: log puts bass in more room",
                Choice(&["log", "lin"]),
                Str,
                fscale
            ),
            field!(
                "Spectrum",
                "Legend",
                "Draw axes and labels",
                Toggle,
                Bool,
                legend
            ),
            field!(
                "Spectrum",
                "Vertical flip",
                "Flip the spectrum upside down",
                Toggle,
                Bool,
                vflip
            ),
            field!(
                "Blend",
                "Blend mode",
                "How the spectrum is combined with the image",
                Choice(BLEND_MODES),
                Str,
                blend_mode
            ),
            field!(
                "Blend",
                "Opacity",
                "Strength of the spectrum layer, 0 to 1",
                Float {
                    min: 0.0,
                    max: 1.0,
                    step: 0.05
                },
                Float,
                opacity
            ),
            field!(
                "x264",
                "Preset",
                "Slower = smaller file at the same quality",
                Choice(X264_PRESETS),
                Str,
                preset
            ),
            field!(
                "x264",
                "CRF",
                "Quality: lower is better, 16-20 is visually lossless",
                Int {
                    min: 0,
                    max: 51,
                    step: 1
                },
                Int,
                crf
            ),
            field!(
                "x264",
                "Profile",
                "H.264 profile",
                Choice(&["high", "main", "baseline"]),
                Str,
                profile
            ),
            field!(
                "x264",
                "B-frames",
                "Max consecutive B-frames",
                Int {
                    min: 0,
                    max: 16,
                    step: 1
                },
                Int,
                bframes
            ),
            field!(
                "x264",
                "GOP size",
                "Keyframe interval in frames",
                Int {
                    min: 1,
                    max: 600,
                    step: 1
                },
                Int,
                gop
            ),
            field!(
                "x264",
                "Threads",
                "Encoder threads (0 = auto, low keeps a Chromebook cool)",
                Int {
                    min: 0,
                    max: 64,
                    step: 1
                },
                Int,
                threads
            ),
            field!(
                "x264",
                "BT.709 tags",
                "Convert to and tag as BT.709 TV range",
                Toggle,
                Bool,
                bt709
            ),
            field!(
                "Audio",
                "Codec",
                "Audio encoder",
                Choice(&["aac", "libopus", "libmp3lame"]),
                Str,
                audio_codec
            ),
            field!(
                "Audio",
                "Bitrate kb/s",
                "Audio bitrate",
                Int {
                    min: 32,
                    max: 512,
                    step: 32
                },
                Int,
                audio_bitrate
            ),
            field!(
                "Audio",
                "Sample rate",
                "Output sample rate (opus needs 48000)",
                Choice(&["48000", "44100", "96000"]),
                Str,
                sample_rate
            ),
            field!(
                "Process",
                "Nice level",
                "CPU priority, 19 = friendliest (Linux only)",
                Int {
                    min: 0,
                    max: 19,
                    step: 1
                },
                Int,
                nice
            ),
            field!(
                "Process",
                "Shortest",
                "Stop when the audio ends",
                Toggle,
                Bool,
                shortest
            ),
            field!(
                "Process",
                "Faststart",
                "Move index to the front for web playback",
                Toggle,
                Bool,
                faststart
            ),
        ]
    }

    pub fn display(&self, s: &Settings) -> String {
        match &self.access {
            Access::Str(get, _) => {
                let v = get(s);
                if v.is_empty() {
                    "<none>".into()
                } else {
                    v.clone()
                }
            }
            Access::Int(get, _) => get(s).to_string(),
            Access::Float(get, _) => fmt_float(*get(s)),
            Access::Bool(get, _) => if *get(s) { "on" } else { "off" }.into(),
        }
    }

    /// Raw text used to pre-fill the edit box.
    pub fn edit_text(&self, s: &Settings) -> String {
        match &self.access {
            Access::Str(get, _) => get(s).clone(),
            _ => self.display(s),
        }
    }

    pub fn is_typeable(&self) -> bool {
        matches!(
            self.kind,
            Kind::Text | Kind::Path(_) | Kind::Int { .. } | Kind::Float { .. }
        )
    }

    /// Step a value left (-1) or right (+1).
    pub fn adjust(&self, s: &mut Settings, dir: i64) {
        match (&self.kind, &self.access) {
            (Kind::Int { min, max, step }, Access::Int(_, get)) => {
                let v = get(s);
                *v = (*v + dir * step).clamp(*min, *max);
            }
            (Kind::Float { min, max, step }, Access::Float(_, get)) => {
                let v = get(s);
                let next = (*v + dir as f64 * step).clamp(*min, *max);
                *v = (next * 100.0).round() / 100.0;
            }
            (Kind::Choice(options), Access::Str(_, get)) => {
                let v = get(s);
                let n = options.len() as i64;
                let idx = options.iter().position(|o| o == v).map_or(0, |i| i as i64);
                *v = options[(idx + dir).rem_euclid(n) as usize].to_string();
            }
            (Kind::Toggle, Access::Bool(_, get)) => {
                let v = get(s);
                *v = !*v;
            }
            _ => {}
        }
    }

    /// Apply typed text. Returns an error message if it does not parse.
    pub fn commit(&self, s: &mut Settings, text: &str) -> Result<(), String> {
        let text = text.trim();
        match (&self.kind, &self.access) {
            (Kind::Int { min, max, .. }, Access::Int(_, get)) => {
                let n: i64 = text
                    .parse()
                    .map_err(|_| format!("'{text}' is not a whole number"))?;
                if n < *min || n > *max {
                    return Err(format!("{} must be between {min} and {max}", self.label));
                }
                *get(s) = n;
            }
            (Kind::Float { min, max, .. }, Access::Float(_, get)) => {
                let n: f64 = text
                    .parse()
                    .map_err(|_| format!("'{text}' is not a number"))?;
                if n < *min || n > *max {
                    return Err(format!("{} must be between {min} and {max}", self.label));
                }
                *get(s) = n;
            }
            (_, Access::Str(_, get)) => *get(s) = text.to_string(),
            _ => {}
        }
        Ok(())
    }
}
