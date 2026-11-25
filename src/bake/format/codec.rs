//! Little-endian byte writer and bounds-checked reader for the `.dynlight` payload.

use std::io::{ErrorKind, Read};

use super::FormatError;

#[derive(Default)]
pub struct Writer {
    pub bytes: Vec<u8>,
}

impl Writer {
    pub fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn f32(&mut self, value: f32) {
        self.u32(value.to_bits());
    }

    pub fn count(&mut self, len: usize) {
        self.u64(len as u64);
    }

    /// A length-prefixed `u32` array.
    pub fn words(&mut self, words: &[u32]) {
        self.count(words.len());
        self.bytes.reserve(words.len() * 4);
        for word in words {
            self.bytes.extend_from_slice(&word.to_le_bytes());
        }
    }
}

/// Words read per `read_exact` call in [`Reader::words`].
const CHUNK_WORDS: usize = 4096;
/// The most words [`Reader::words`] reserves up front, so a corrupt length can't allocate more
/// than this before the data runs out.
const PREALLOCATED_WORDS: usize = 1 << 26;

/// Bounds-checked little-endian reader over any byte source: a slice, or a decompressor
/// streaming the payload. `remaining` is the payload length still to come.
pub struct Reader<R> {
    inner: R,
    remaining: u64,
}

impl<'a> Reader<&'a [u8]> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self::with_len(bytes, bytes.len() as u64)
    }
}

impl<R: Read> Reader<R> {
    /// Reads at most `len` bytes from `inner`.
    pub fn with_len(inner: R, len: u64) -> Self {
        Self {
            inner,
            remaining: len,
        }
    }

    pub fn remaining(&self) -> u64 {
        self.remaining
    }

    pub fn into_inner(self) -> R {
        self.inner
    }

    fn fill(&mut self, buf: &mut [u8]) -> Result<(), FormatError> {
        if buf.len() as u64 > self.remaining {
            return Err(FormatError::Truncated);
        }
        self.inner
            .read_exact(buf)
            .map_err(|error| match error.kind() {
                ErrorKind::UnexpectedEof => FormatError::Truncated,
                _ => FormatError::Decompress(error.to_string()),
            })?;
        self.remaining -= buf.len() as u64;
        Ok(())
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], FormatError> {
        let mut bytes = [0; N];
        self.fill(&mut bytes)?;
        Ok(bytes)
    }

    pub fn u32(&mut self) -> Result<u32, FormatError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64, FormatError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub fn f32(&mut self) -> Result<f32, FormatError> {
        Ok(f32::from_bits(self.u32()?))
    }

    /// A count of items that each take at least `item_bytes`; rejects counts the remaining data
    /// can't hold, so corrupt files can't trigger huge allocations.
    pub fn count(&mut self, item_bytes: usize) -> Result<usize, FormatError> {
        let len = self.u64()?;
        let fits = usize::try_from(len).ok().filter(|&len| {
            (len as u64)
                .checked_mul(item_bytes.max(1) as u64)
                .is_some_and(|b| b <= self.remaining)
        });
        fits.ok_or(FormatError::Truncated)
    }

    pub fn words(&mut self) -> Result<Vec<u32>, FormatError> {
        let len = self.count(4)?;
        let mut words = Vec::with_capacity(len.min(PREALLOCATED_WORDS));
        let mut chunk = [0u8; CHUNK_WORDS * 4];
        while words.len() < len {
            let take = (len - words.len()).min(CHUNK_WORDS);
            let bytes = &mut chunk[..take * 4];
            self.fill(bytes)?;
            words.extend(
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|&b| u32::from_le_bytes(b)),
            );
        }
        Ok(words)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip() {
        let mut writer = Writer::default();
        writer.u32(7);
        writer.u64(u64::MAX - 1);
        writer.f32(-1.5);
        writer.words(&[1, 2, 0xDEAD_BEEF]);
        let mut reader = Reader::new(&writer.bytes);
        assert_eq!(reader.u32(), Ok(7));
        assert_eq!(reader.u64(), Ok(u64::MAX - 1));
        assert_eq!(reader.f32(), Ok(-1.5));
        assert_eq!(reader.words(), Ok(vec![1, 2, 0xDEAD_BEEF]));
        assert_eq!(reader.remaining(), 0);
        assert_eq!(reader.u32(), Err(FormatError::Truncated));
    }

    #[test]
    fn absurd_lengths_are_rejected_before_allocating() {
        let mut writer = Writer::default();
        writer.u64(u64::MAX / 2);
        assert_eq!(
            Reader::new(&writer.bytes).words(),
            Err(FormatError::Truncated)
        );
    }
}
