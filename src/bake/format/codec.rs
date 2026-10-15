//! Little-endian byte writer and bounds-checked reader for the `.dynlight` payload.

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

pub struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], FormatError> {
        if count > self.remaining() {
            return Err(FormatError::Truncated);
        }
        let bytes = &self.bytes[self.offset..self.offset + count];
        self.offset += count;
        Ok(bytes)
    }

    pub fn u32(&mut self) -> Result<u32, FormatError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn u64(&mut self) -> Result<u64, FormatError> {
        let low = u64::from(self.u32()?);
        let high = u64::from(self.u32()?);
        Ok(low | high << 32)
    }

    pub fn f32(&mut self) -> Result<f32, FormatError> {
        Ok(f32::from_bits(self.u32()?))
    }

    /// A count of items that each take at least `item_bytes`; rejects counts the remaining data
    /// can't hold, so corrupt files can't trigger huge allocations.
    pub fn count(&mut self, item_bytes: usize) -> Result<usize, FormatError> {
        let len = self.u64()?;
        let fits = usize::try_from(len).ok().filter(|&len| {
            len.checked_mul(item_bytes.max(1))
                .is_some_and(|b| b <= self.remaining())
        });
        fits.ok_or(FormatError::Truncated)
    }

    pub fn words(&mut self) -> Result<Vec<u32>, FormatError> {
        let len = self.count(4)?;
        let bytes = self.take(len * 4)?;
        Ok(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&b| u32::from_le_bytes(b))
            .collect())
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
