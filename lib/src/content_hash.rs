//! Portable, stable hashing suitable for identifying values

// Re-export DigestUpdate so that the ContentHash proc macro can be used in
// external crates without directly depending on the digest crate.
pub use digest::Update as DigestUpdate;
pub use jj_core::content_hash::ContentHash;
pub use jj_core::content_hash::blake2b_hash;

#[cfg(test)]
mod test {

    use blake2::Blake2b512;

    use super::*;
    use crate::hex_util;

    // TODO: move this over when we lower `hex_util.rs`
    #[test]
    fn test_consistent_hashing() {
        #[derive(ContentHash)]
        struct Foo {
            x: Vec<Option<i32>>,
            y: i64,
        }
        let foo_hash = hex_util::encode_hex(&hash(&Foo {
            x: vec![None, Some(42)],
            y: 17,
        }));
        insta::assert_snapshot!(
            foo_hash,
            @"e33c423b4b774b1353c414e0f9ef108822fde2fd5113fcd53bf7bd9e74e3206690b96af96373f268ed95dd020c7cbe171c7b7a6947fcaf5703ff6c8e208cefd4"
        );

        // Try again with an equivalent generic struct deriving ContentHash.
        #[derive(ContentHash)]
        struct GenericFoo<X, Y> {
            x: X,
            y: Y,
        }
        assert_eq!(
            hex_util::encode_hex(&hash(&GenericFoo {
                x: vec![None, Some(42)],
                y: 17i64
            })),
            foo_hash
        );
    }

    fn hash(x: &(impl ContentHash + ?Sized)) -> digest::Output<Blake2b512> {
        blake2b_hash(x)
    }
}
