const INITIAL_STATE: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

const ROUND_CONSTANTS: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

pub(crate) struct ContentFingerprint {
    first: u64,
    second: u64,
    bytes: u64,
}

impl ContentFingerprint {
    pub(crate) const fn new() -> Self {
        Self {
            first: 0xcbf2_9ce4_8422_2325,
            second: 0x9e37_79b9_7f4a_7c15,
            bytes: 0,
        }
    }

    pub(crate) fn write(&mut self, input: &[u8]) {
        for byte in input {
            self.first ^= u64::from(*byte);
            self.first = self.first.wrapping_mul(0x0000_0100_0000_01b3);
            self.second ^= u64::from(*byte).wrapping_add(self.bytes);
            self.second = self
                .second
                .rotate_left(13)
                .wrapping_mul(0x9e37_79b1_85eb_ca87);
            self.bytes = self.bytes.wrapping_add(1);
        }
    }

    pub(crate) fn finish(self) -> String {
        let first = mix64(self.first ^ self.bytes);
        let second = mix64(self.second ^ self.bytes.rotate_left(29));
        format!("fp128:{first:016x}{second:016x}")
    }
}

const fn mix64(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

pub(crate) struct FingerprintHasher {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_len: usize,
    bytes: u64,
}

impl FingerprintHasher {
    pub(crate) const fn new() -> Self {
        Self {
            state: INITIAL_STATE,
            buffer: [0; 64],
            buffer_len: 0,
            bytes: 0,
        }
    }

    pub(crate) fn write(&mut self, mut input: &[u8]) {
        self.bytes = self.bytes.wrapping_add(input.len() as u64);
        if self.buffer_len != 0 {
            let needed = 64 - self.buffer_len;
            let copied = needed.min(input.len());
            self.buffer[self.buffer_len..self.buffer_len + copied]
                .copy_from_slice(&input[..copied]);
            self.buffer_len += copied;
            input = &input[copied..];
            if self.buffer_len == 64 {
                compress(&mut self.state, &self.buffer);
                self.buffer_len = 0;
            } else {
                return;
            }
        }
        while input.len() >= 64 {
            let block: &[u8; 64] = input[..64].try_into().expect("SHA-256 block length");
            compress(&mut self.state, block);
            input = &input[64..];
        }
        self.buffer[..input.len()].copy_from_slice(input);
        self.buffer_len = input.len();
    }

    pub(crate) fn finish(mut self) -> String {
        let bit_len = self.bytes.wrapping_mul(8);
        self.buffer[self.buffer_len] = 0x80;
        self.buffer_len += 1;
        if self.buffer_len > 56 {
            self.buffer[self.buffer_len..].fill(0);
            compress(&mut self.state, &self.buffer);
            self.buffer = [0; 64];
        } else {
            self.buffer[self.buffer_len..56].fill(0);
        }
        self.buffer[56..].copy_from_slice(&bit_len.to_be_bytes());
        compress(&mut self.state, &self.buffer);
        let mut output = String::with_capacity(71);
        output.push_str("sha256:");
        for word in self.state {
            use std::fmt::Write as _;
            write!(output, "{word:08x}").expect("writing to String cannot fail");
        }
        output
    }
}

#[cfg(test)]
pub(crate) fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = FingerprintHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

#[allow(clippy::many_single_char_names)]
fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut schedule = [0_u32; 64];
    for (index, word) in block.chunks_exact(4).enumerate() {
        schedule[index] = u32::from_be_bytes(word.try_into().expect("SHA-256 word length"));
    }
    for index in 16..64 {
        let s0 = schedule[index - 15].rotate_right(7)
            ^ schedule[index - 15].rotate_right(18)
            ^ (schedule[index - 15] >> 3);
        let s1 = schedule[index - 2].rotate_right(17)
            ^ schedule[index - 2].rotate_right(19)
            ^ (schedule[index - 2] >> 10);
        schedule[index] = schedule[index - 16]
            .wrapping_add(s0)
            .wrapping_add(schedule[index - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for index in 0..64 {
        let upper = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choose = (e & f) ^ (!e & g);
        let temporary1 = h
            .wrapping_add(upper)
            .wrapping_add(choose)
            .wrapping_add(ROUND_CONSTANTS[index])
            .wrapping_add(schedule[index]);
        let lower = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let temporary2 = lower.wrapping_add(majority);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temporary1);
        d = c;
        c = b;
        b = a;
        a = temporary1.wrapping_add(temporary2);
    }
    for (target, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *target = target.wrapping_add(value);
    }
}

#[cfg(test)]
mod tests {
    use super::{ContentFingerprint, FingerprintHasher, hash_bytes};

    #[test]
    fn matches_sha256_known_vectors_and_streaming() {
        assert_eq!(
            hash_bytes(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash_bytes(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let mut streamed = FingerprintHasher::new();
        streamed.write(b"a");
        streamed.write(b"b");
        streamed.write(b"c");
        assert_eq!(streamed.finish(), hash_bytes(b"abc"));
    }

    #[test]
    fn whole_content_fingerprint_is_streaming_and_change_sensitive() {
        let mut whole = ContentFingerprint::new();
        whole.write(b"abcdef");
        let mut streamed = ContentFingerprint::new();
        streamed.write(b"ab");
        streamed.write(b"cd");
        streamed.write(b"ef");
        assert_eq!(whole.finish(), streamed.finish());

        let mut changed = ContentFingerprint::new();
        changed.write(b"abcdeg");
        let mut original = ContentFingerprint::new();
        original.write(b"abcdef");
        assert_ne!(original.finish(), changed.finish());
    }
}
