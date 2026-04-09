pub mod detection;
pub mod mapping;
pub mod region;
pub mod sql;

pub use detection::detect_embedded_regions;
pub use region::EmbeddedLanguage;
pub use region::EmbeddedRegion;
