//! Rewriting a [`ShaderBuffer`] asset in place, for buffers that change every frame.

use bevy::asset::RenderAssetUsages;
use bevy::render::storage::ShaderBuffer;

/// A buffer asset for [`write_shader_buffer`] holding `bytes`. Only the render world keeps the
/// data, so a change moves it there instead of copying it.
pub fn frame_shader_buffer(bytes: &[u8], label: &'static str) -> ShaderBuffer {
    let mut buffer = ShaderBuffer::with_size(0, RenderAssetUsages::RENDER_WORLD);
    buffer.label = label.into();
    write_shader_buffer(&mut buffer, &[bytes], 0);
    buffer
}

/// The GPU size for `len` bytes of contents in a buffer of `current` bytes: unchanged while they
/// fit, else the next power of two. A buffer that keeps its size keeps its GPU buffer, and the
/// materials binding it keep their bind groups.
pub fn buffer_capacity(current: usize, len: usize) -> usize {
    if len <= current {
        current
    } else {
        len.next_power_of_two()
    }
}

/// Replaces the contents of `buffer` with `parts`, one after another, zero-padded to at least
/// `min_len` bytes and to a multiple of 16. Bytes past the contents keep stale data, so shaders
/// must not size their reads from the binding.
pub fn write_shader_buffer(buffer: &mut ShaderBuffer, parts: &[&[u8]], min_len: usize) {
    let len: usize = parts.iter().map(|part| part.len()).sum();
    let padded = len.max(min_len).next_multiple_of(16);
    let capacity = buffer_capacity(buffer.buffer_size() as usize, padded);
    buffer.clear();
    for part in parts {
        buffer.extend_from_slice(part);
    }
    buffer.extend((len..padded).map(|_| 0u8));
    buffer.resize_buffer(capacity as u64);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::storage::ShaderBufferData;

    fn contents(buffer: &ShaderBuffer) -> &[u8] {
        match &buffer.data {
            ShaderBufferData::Initialized { data, .. } => &data[..],
            ShaderBufferData::Uninitialized(_) => panic!("the buffer holds no data"),
        }
    }

    #[test]
    fn parts_are_joined_and_padded() {
        let mut buffer = frame_shader_buffer(&[], "test");
        write_shader_buffer(&mut buffer, &[&[1, 2, 3], &[4]], 20);
        let mut expected = vec![1, 2, 3, 4];
        expected.resize(32, 0);
        assert_eq!(contents(&buffer), expected.as_slice());
        assert_eq!(buffer.buffer_size(), 32);
    }

    #[test]
    fn the_gpu_size_only_grows() {
        let mut buffer = frame_shader_buffer(&[0; 48], "test");
        assert_eq!(buffer.buffer_size(), 64);
        write_shader_buffer(&mut buffer, &[&[7; 16]], 0);
        assert_eq!(contents(&buffer), &[7; 16]);
        assert_eq!(buffer.buffer_size(), 64);
        write_shader_buffer(&mut buffer, &[&[7; 80]], 0);
        assert_eq!(buffer.buffer_size(), 128);
    }
}
