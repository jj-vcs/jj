//! Portable, stable hashing suitable for identifying values

// Re-export DigestUpdate so that the ContentHash proc macro can be used in
// external crates without directly depending on the digest crate.
pub use digest::Update as DigestUpdate;
pub use jj_core::content_hash::ContentHash;
pub use jj_core::content_hash::blake2b_hash;
