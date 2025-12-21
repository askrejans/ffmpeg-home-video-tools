// Processing module - contains all video processing operations
pub mod convert;
pub mod pad;
pub mod crop;
pub mod audio;
pub mod concat;

// Re-export common types
pub use convert::convert_videos;
pub use pad::pad_videos;
pub use crop::crop_videos;
pub use audio::resample_audio;
pub use concat::concatenate_videos;
