use sha2::{Digest, Sha256};

pub struct MerkleTree {
    pub leaves: Vec<String>,
    pub root_hex: String,
}

impl MerkleTree {
    pub fn new(leaves: Vec<String>) -> Self {
        if leaves.is_empty() {
            return MerkleTree {
                leaves: vec![],
                root_hex: "".to_string(),
            };
        }

        let mut current_hashes = leaves.clone();

        while current_hashes.len() > 1 {
            let mut next_level = Vec::new();
            for chunk in current_hashes.chunks(2) {
                if chunk.len() == 2 {
                    let mut hasher = Sha256::new();
                    hasher.update(&chunk[0]);
                    hasher.update(&chunk[1]);
                    next_level.push(hex::encode(hasher.finalize()));
                } else {
                    // Odd number of leaves: repeat the last one
                    let mut hasher = Sha256::new();
                    hasher.update(&chunk[0]);
                    hasher.update(&chunk[0]);
                    next_level.push(hex::encode(hasher.finalize()));
                }
            }
            current_hashes = next_level;
        }

        MerkleTree {
            leaves,
            root_hex: current_hashes[0].clone(),
        }
    }
}
