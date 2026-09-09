pub mod cache;
pub mod census;
pub mod inputs;
pub mod platform;
pub mod tree;

use sha2::{Digest, Sha256};

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
