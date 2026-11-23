//! Streamed `.dynlight` decoding: the gzip stream is inflated and parsed section by section, so
//! neither the packed file nor the whole unpacked payload is ever held.

use std::io::{self, Read};

use flate2::read::GzDecoder;

use super::codec::Reader;
use super::types::BakedScene;
use super::{FormatError, HEADER_BYTES, Header, parse_header, parse_payload};

/// Hashes the bytes passing through, and counts them.
struct Checked<R> {
    inner: R,
    hasher: crc32fast::Hasher,
    count: u64,
}

impl<R: Read> Read for Checked<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.hasher.update(&buf[..read]);
        self.count += read as u64;
        Ok(read)
    }
}

/// Parses a `.dynlight` file from `source` as it is read. The result equals
/// [`decode`](super::decode)'s; peak memory is the parsed scene plus small buffers.
///
/// The checksum and length can only be checked once the payload has been read, so a corrupt file
/// may fail with a parse error where [`decode`](super::decode) says checksum mismatch; this
/// reads the rest of the payload in that case to report the same error.
pub fn decode_stream(source: impl Read) -> Result<BakedScene, FormatError> {
    let mut source = source;
    let mut head = Vec::with_capacity(HEADER_BYTES);
    (&mut source)
        .take(HEADER_BYTES as u64)
        .read_to_end(&mut head)
        .map_err(|error| FormatError::Decompress(error.to_string()))?;
    let Header { length, checksum } = parse_header(&head)?;

    let checked = Checked {
        inner: GzDecoder::new(io::BufReader::with_capacity(1 << 16, source)),
        hasher: crc32fast::Hasher::new(),
        count: 0,
    };
    let mut reader = Reader::with_len(checked, length);
    let parsed = parse_payload(&mut reader);
    if matches!(parsed, Err(FormatError::Decompress(_))) {
        return parsed;
    }
    let mut checked = reader.into_inner();
    let finished = match &parsed {
        // One byte past the declared length must not exist.
        Ok(_) => io::copy(&mut (&mut checked).take(1), &mut io::sink()).map(|_| ()),
        Err(_) => io::copy(&mut checked, &mut io::sink()).map(|_| ()),
    };
    if let Err(error) = finished
        && parsed.is_ok()
    {
        return Err(FormatError::Decompress(error.to_string()));
    }
    if checked.count != length {
        return Err(FormatError::Truncated);
    }
    if checked.hasher.finalize() != checksum {
        return Err(FormatError::ChecksumMismatch);
    }
    parsed
}
