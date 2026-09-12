#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

const BITS_PER_KEY: usize = 10;
const BYTES_PER_BLOCK: usize = 32;

// 8 distinct prime multipliers for the 8 × 32-bit lanes
const SALTS: [u32; 8] = [
    0x47b6137b, 0x44974d91, 0x8824ad5b, 0xa2b7289d,
    0x705495c7, 0x2df1424b, 0x9efc4947, 0x5c6bfb31,
];

#[inline]
fn hash64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xc6a4a7935bd1e995;
    for chunk in data.chunks(8) {
        let mut val = 0u64;
        for (i, &b) in chunk.iter().enumerate() {
            val |= (b as u64) << (i * 8);
        }
        h ^= val;
        h = h.wrapping_mul(0x5bd1e9955bd1e995);
        h ^= h >> 47;
    }
    h
}

pub struct BloomBuilder {
    keys: Vec<u64>,
}

impl BloomBuilder {
    pub fn new() -> Self {
        Self { keys: Vec::new() }
    }

    pub fn add(&mut self, key: &[u8]) {
        self.keys.push(hash64(key));
    }

    pub fn build(self) -> Vec<u8> {
        let n_keys = self.keys.len();
        if n_keys == 0 {
            return Vec::new();
        }

        let num_blocks = ((n_keys * BITS_PER_KEY + 255) / 256).max(1);
        let mut data = vec![0u8; num_blocks * BYTES_PER_BLOCK];

        for h in self.keys {
            let block_idx = ((h >> 32) as usize) % num_blocks;
            let block_offset = block_idx * BYTES_PER_BLOCK;
            let h32 = h as u32;

            let block_slice = &mut data[block_offset..block_offset + BYTES_PER_BLOCK];
            Self::insert_block(block_slice, h32);
        }

        data
    }

    fn insert_block(block: &mut [u8], h: u32) {
        for i in 0..8 {
            let rot = h.wrapping_mul(SALTS[i]) >> 27;
            let bit_mask = 1u32 << rot;
            let byte_idx = i * 4;
            let word = u32::from_le_bytes(block[byte_idx..byte_idx + 4].try_into().unwrap());
            let updated = word | bit_mask;
            block[byte_idx..byte_idx + 4].copy_from_slice(&updated.to_le_bytes());
        }
    }
}

pub struct BloomFilter<'a> {
    data: &'a [u8],
    num_blocks: usize,
}

impl<'a> BloomFilter<'a> {
    pub fn new(data: &'a [u8]) -> Option<Self> {
        if data.is_empty() || data.len() % BYTES_PER_BLOCK != 0 {
            return None;
        }
        let num_blocks = data.len() / BYTES_PER_BLOCK;
        Some(Self { data, num_blocks })
    }

    pub fn contains(&self, key: &[u8]) -> bool {
        let h = hash64(key);
        let block_idx = ((h >> 32) as usize) % self.num_blocks;
        let block_offset = block_idx * BYTES_PER_BLOCK;
        let block = &self.data[block_offset..block_offset + BYTES_PER_BLOCK];
        let h32 = h as u32;

        #[cfg(target_arch = "x86_64")]
        {
            if is_x86_feature_detected!("avx2") {
                // SAFETY: avx2 feature verified dynamically by runtime detection.
                return unsafe { self.contains_avx2(block, h32) };
            }
        }

        self.contains_scalar(block, h32)
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn contains_avx2(&self, block: &[u8], h: u32) -> bool {
        // SAFETY: block is guaranteed to be 32 bytes by constructor invariant.
        let block_vec = _mm256_loadu_si256(block.as_ptr() as *const __m256i);
        let hash_vec = _mm256_set1_epi32(h as i32);
        let salts_vec = _mm256_loadu_si256(SALTS.as_ptr() as *const __m256i);

        let multiplied = _mm256_mullo_epi32(hash_vec, salts_vec);
        let shifted = _mm256_srli_epi32(multiplied, 27);
        let ones = _mm256_set1_epi32(1);
        let mask = _mm256_sllv_epi32(ones, shifted);

        // _mm256_testc_si256 returns 1 if (NOT block_vec AND mask) == 0
        // which proves every set bit in mask is present in block_vec.
        _mm256_testc_si256(block_vec, mask) != 0
    }

    fn contains_scalar(&self, block: &[u8], h: u32) -> bool {
        for i in 0..8 {
            let rot = h.wrapping_mul(SALTS[i]) >> 27;
            let bit_mask = 1u32 << rot;
            let byte_idx = i * 4;
            let word = u32::from_le_bytes(block[byte_idx..byte_idx + 4].try_into().unwrap());
            if (word & bit_mask) != bit_mask {
                return false;
            }
        }
        true
    }
}