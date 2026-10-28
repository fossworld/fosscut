use serde::{Deserialize, Serialize};

/// The YUV to RGB matrix of a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Matrix {
    /// ITU-R BT.601 (standard definition).
    Bt601,
    /// ITU-R BT.709 (high definition).
    Bt709,
    /// ITU-R BT.2020 non-constant luminance.
    Bt2020Ncl,
    /// RGB stored directly (no matrix).
    Identity,
    /// Not tagged in the file.
    Unspecified,
}

/// The code value range of a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Range {
    /// Limited ("TV" or "video") range: luma 16 to 235 for 8-bit.
    Limited,
    /// Full ("PC" or "JPEG") range: 0 to 255 for 8-bit.
    Full,
    /// Not tagged in the file.
    Unspecified,
}

/// The color primaries of a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Primaries {
    /// BT.709 and sRGB primaries.
    Bt709,
    /// BT.601 525-line (SMPTE 170M) primaries.
    Bt601Ntsc,
    /// BT.601 625-line (BT.470 BG) primaries.
    Bt601Pal,
    /// BT.2020 wide-gamut primaries.
    Bt2020,
    /// Display P3 primaries (SMPTE EG 432).
    DisplayP3,
    /// Not tagged in the file.
    Unspecified,
}

/// The transfer function (opto-electronic characteristic) of a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transfer {
    /// BT.709 (also used by BT.601 and BT.2020 SDR).
    Bt709,
    /// The sRGB curve (IEC 61966-2-1).
    Srgb,
    /// Linear light.
    Linear,
    /// SMPTE ST 2084 perceptual quantizer (HDR10).
    Pq,
    /// ARIB STD-B67 hybrid log-gamma.
    Hlg,
    /// Not tagged in the file.
    Unspecified,
}

/// Color tags of a stream as stored in the file (ADR-0009, item 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorInfo {
    /// YUV to RGB matrix.
    pub matrix: Matrix,
    /// Code value range.
    pub range: Range,
    /// Color primaries.
    pub primaries: Primaries,
    /// Transfer function.
    pub transfer: Transfer,
}

impl ColorInfo {
    /// Fills untagged values with the conventional defaults, so the color pipeline never
    /// guesses: BT.601 matrix and primaries below 720 lines and BT.709 from 720 lines up,
    /// limited range, BT.709 transfer. These are the assumptions players make for
    /// untagged SDR video.
    pub fn resolved(self, height: u32) -> ColorInfo {
        let hd = height >= 720;
        ColorInfo {
            matrix: match self.matrix {
                Matrix::Unspecified if hd => Matrix::Bt709,
                Matrix::Unspecified => Matrix::Bt601,
                other => other,
            },
            range: match self.range {
                Range::Unspecified => Range::Limited,
                other => other,
            },
            primaries: match self.primaries {
                Primaries::Unspecified if hd => Primaries::Bt709,
                Primaries::Unspecified => Primaries::Bt601Ntsc,
                other => other,
            },
            transfer: match self.transfer {
                Transfer::Unspecified => Transfer::Bt709,
                other => other,
            },
        }
    }
}

impl Default for ColorInfo {
    fn default() -> ColorInfo {
        ColorInfo {
            matrix: Matrix::Unspecified,
            range: Range::Unspecified,
            primaries: Primaries::Unspecified,
            transfer: Transfer::Unspecified,
        }
    }
}
