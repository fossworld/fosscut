use thiserror::Error;

/// Errors from planning and rendering frames.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum RenderError {
    /// No GPU adapter is available, not even a software one.
    #[error("no GPU adapter is available: {message}")]
    NoAdapter {
        /// What wgpu reported.
        message: String,
    },
    /// The GPU device could not be created or was lost.
    #[error("the GPU device failed: {message}")]
    Device {
        /// What wgpu reported.
        message: String,
    },
    /// The timeline to render does not exist.
    #[error("timeline {timeline} does not exist")]
    TimelineNotFound {
        /// The requested timeline id.
        timeline: String,
    },
    /// A layer's source frame could not be produced.
    #[error("no frame for asset {asset}: {message}")]
    MissingFrame {
        /// The asset whose frame was requested.
        asset: String,
        /// Why the frame is missing.
        message: String,
    },
    /// A frame uses a color encoding the pipeline does not handle yet.
    #[error("unsupported color encoding: {message}")]
    UnsupportedColor {
        /// What is not supported.
        message: String,
    },
    /// A 3D LUT could not be parsed.
    #[error("invalid LUT: {message}")]
    InvalidLut {
        /// What is wrong with it.
        message: String,
    },
    /// Reading the rendered image back from the GPU failed.
    #[error("reading the frame back failed: {message}")]
    Readback {
        /// What wgpu reported.
        message: String,
    },
}

impl RenderError {
    /// The stable error code used in FossCut API responses (ADR-0006).
    pub fn code(&self) -> &'static str {
        match self {
            RenderError::NoAdapter { .. } => "fosscut.render.no_adapter",
            RenderError::Device { .. } => "fosscut.render.device_failed",
            RenderError::TimelineNotFound { .. } => "fosscut.timeline.not_found",
            RenderError::MissingFrame { .. } => "fosscut.render.missing_frame",
            RenderError::UnsupportedColor { .. } => "fosscut.render.unsupported_color",
            RenderError::InvalidLut { .. } => "fosscut.render.invalid_lut",
            RenderError::Readback { .. } => "fosscut.render.readback_failed",
        }
    }
}
