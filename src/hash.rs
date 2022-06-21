use meowhash::*;

pub fn hash_proxy<H: std::hash::Hash, F: FnMut(&[u8])>(h: &H, f: F) {
    struct Hasher<F: FnMut(&[u8])> {
        f: F,
    }
    impl<F: FnMut(&[u8])> std::hash::Hasher for Hasher<F> {
        fn finish(&self) -> u64 {
            0
        }
        fn write(&mut self, bytes: &[u8]) {
            (self.f)(bytes);
        }
    }
    let mut proxy = Hasher { f };
    h.hash(&mut proxy)
}

pub fn meow_hash<H: std::hash::Hash>(seed: Option<MeowHash>, h: &H) -> u128 {
    use digest::Digest;
    let mut hasher = MeowHasher::with_seed(seed.unwrap_or_else(MeowHash::default_seed));
    hash_proxy(h, |bytes| hasher.update(bytes));
    hasher.finalise().as_u128()
}
