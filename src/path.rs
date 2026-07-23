pub(crate) fn looks_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}

pub(crate) struct RevisionHasher(u64);

impl RevisionHasher {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    pub(crate) const fn new() -> Self {
        Self(Self::OFFSET)
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    pub(crate) fn finish(self) -> String {
        format!("fnv1a64:{:016x}", self.0)
    }
}
