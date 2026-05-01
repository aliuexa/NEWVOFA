use std::mem;

const FRAME_TAIL: [u8; 4] = [0x00, 0x00, 0x80, 0x7F];

pub struct JustFloatParser {
    buffer: Vec<u8>,
}

impl JustFloatParser {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(4096),
        }
    }

    pub fn feed_bytes(&mut self, bytes: &[u8]) -> Vec<Vec<f32>> {
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();

        loop {
            let tail_pos = self.find_tail();
            let Some(pos) = tail_pos else {
                break;
            };

            if pos % mem::size_of::<f32>() == 0 && pos > 0 {
                let float_count = pos / mem::size_of::<f32>();
                let mut frame = Vec::with_capacity(float_count);
                for chunk in self.buffer[..pos].chunks_exact(mem::size_of::<f32>()) {
                    let arr: [u8; 4] = chunk.try_into().unwrap();
                    frame.push(f32::from_le_bytes(arr));
                }
                frames.push(frame);
                self.buffer.drain(..pos + FRAME_TAIL.len());
            } else {
                self.buffer.drain(..1);
            }
        }

        self.trim_stale_data();
        frames
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
    }

    fn find_tail(&self) -> Option<usize> {
        if self.buffer.len() < FRAME_TAIL.len() {
            return None;
        }
        self.buffer
            .windows(FRAME_TAIL.len())
            .position(|window| window == FRAME_TAIL)
    }

    fn trim_stale_data(&mut self) {
        let max_buffer = 8192;
        if self.buffer.len() > max_buffer {
            let excess = self.buffer.len() - max_buffer;
            self.buffer.drain(..excess);
        }
    }
}

impl Default for JustFloatParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_frame() {
        let mut parser = JustFloatParser::new();
        let mut data = Vec::new();
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&2.0f32.to_le_bytes());
        data.extend_from_slice(&3.0f32.to_le_bytes());
        data.extend_from_slice(&FRAME_TAIL);

        let frames = parser.feed_bytes(&data);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_multiple_frames() {
        let mut parser = JustFloatParser::new();
        let mut data = Vec::new();
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&FRAME_TAIL);
        data.extend_from_slice(&2.0f32.to_le_bytes());
        data.extend_from_slice(&3.0f32.to_le_bytes());
        data.extend_from_slice(&FRAME_TAIL);

        let frames = parser.feed_bytes(&data);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], vec![1.0]);
        assert_eq!(frames[1], vec![2.0, 3.0]);
    }

    #[test]
    fn test_partial_feed() {
        let mut parser = JustFloatParser::new();
        let mut data = Vec::new();
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&2.0f32.to_le_bytes());

        let frames = parser.feed_bytes(&data[..4]);
        assert!(frames.is_empty());

        let frames = parser.feed_bytes(&data[4..]);
        assert!(frames.is_empty());

        let frames = parser.feed_bytes(&FRAME_TAIL);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], vec![1.0, 2.0]);
    }

    #[test]
    fn test_empty_feed() {
        let mut parser = JustFloatParser::new();
        let frames = parser.feed_bytes(&[]);
        assert!(frames.is_empty());
    }
}
