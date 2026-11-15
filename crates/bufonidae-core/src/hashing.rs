//! Content hashes (BLAKE3), recorded when a file is stored and checked whenever
//! a copy is read back.
//!
//! Mega gives no content hash through rclone, so this is the only way to tell a
//! damaged copy from a good one: the hash is of the file as it was **sent**.

use std::io::Read as _;
use std::path::Path;

/// Hex BLAKE3 of the file at `path`, read in chunks (files can be huge).
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_match_the_reference_and_differ_for_different_content() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::write(&a, b"hello").unwrap();
        std::fs::write(&b, b"hellp").unwrap();
        assert_eq!(
            hash_file(&a).unwrap(),
            blake3::hash(b"hello").to_hex().to_string()
        );
        assert_ne!(hash_file(&a).unwrap(), hash_file(&b).unwrap());
        assert!(hash_file(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn a_file_larger_than_the_chunk_hashes_the_same_as_in_one_piece() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big");
        let data: Vec<u8> = (0..(3 << 20) + 17).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        assert_eq!(
            hash_file(&path).unwrap(),
            blake3::hash(&data).to_hex().to_string()
        );
    }
}
