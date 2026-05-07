use std::mem;

const JUST_FLOAT_TAIL: [u8; 4] = [0x00, 0x00, 0x80, 0x7F];
const PARAMETER_TAIL: [u8; 4] = [0x00, 0x00, 0x90, 0x7F];

#[derive(Debug, Clone)]
pub struct ParameterEntry {
    pub name: String,
    pub value: f32,
}

#[derive(Debug, Clone)]
pub enum Frame {
    JustFloat(Vec<f32>),
    Parameter(Vec<ParameterEntry>),
}

pub struct UnifiedParser {
    buffer: Vec<u8>,
}

impl UnifiedParser {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(4096),
        }
    }

    pub fn feed_bytes(&mut self, bytes: &[u8]) -> Vec<Frame> {
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();

        loop {
            let result = self.find_first_tail();
            let Some((tail_pos, tail_type)) = result else {
                break;
            };

            let tail_len = 4;
            let payload = &self.buffer[..tail_pos];

            match tail_type {
                TailType::JustFloat => {
                    if tail_pos % mem::size_of::<f32>() == 0 && tail_pos > 0 {
                        let float_count = tail_pos / mem::size_of::<f32>();
                        let mut frame = Vec::with_capacity(float_count);
                        for chunk in payload.chunks_exact(mem::size_of::<f32>()) {
                            let arr: [u8; 4] = chunk.try_into().unwrap();
                            frame.push(f32::from_le_bytes(arr));
                        }
                        frames.push(Frame::JustFloat(frame));
                        self.buffer.drain(..tail_pos + tail_len);
                    } else {
                        self.buffer.drain(..1);
                    }
                }
                TailType::Parameter => {
                    if tail_pos == 0 {
                        self.buffer.drain(..tail_len);
                        continue;
                    }
                    let entries = Self::parse_parameter_payload(payload);
                    frames.push(Frame::Parameter(entries));
                    self.buffer.drain(..tail_pos + tail_len);
                }
            }
        }

        self.trim_stale_data();
        frames
    }

    fn parse_parameter_payload(data: &[u8]) -> Vec<ParameterEntry> {
        let mut entries = Vec::new();
        let mut offset = 0usize;
        let len = data.len();

        while offset < len {
            let null_pos = data[offset..].iter().position(|&b| b == 0x00);
            let Some(name_end) = null_pos else {
                break;
            };
            let name_bytes = &data[offset..offset + name_end];
            if name_bytes.is_empty() {
                offset = offset + name_end + 1;
                continue;
            }
            let name = String::from_utf8_lossy(name_bytes).to_string();
            offset = offset + name_end + 1;

            if offset + mem::size_of::<f32>() > len {
                break;
            }
            let arr: [u8; 4] = data[offset..offset + mem::size_of::<f32>()]
                .try_into()
                .unwrap();
            let value = f32::from_le_bytes(arr);
            offset += mem::size_of::<f32>();

            entries.push(ParameterEntry { name, value });
        }

        entries
    }

    fn find_first_tail(&self) -> Option<(usize, TailType)> {
        if self.buffer.len() < 4 {
            return None;
        }

        let mut best: Option<(usize, TailType)> = None;

        for (pos, window) in self.buffer.windows(4).enumerate() {
            if window == JUST_FLOAT_TAIL {
                best = Some((pos, TailType::JustFloat));
                break;
            }
            if window == PARAMETER_TAIL {
                best = Some((pos, TailType::Parameter));
                break;
            }
        }

        best
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
    }

    fn trim_stale_data(&mut self) {
        let max_buffer = 8192;
        if self.buffer.len() > max_buffer {
            let excess = self.buffer.len() - max_buffer;
            self.buffer.drain(..excess);
        }
    }
}

impl Default for UnifiedParser {
    fn default() -> Self {
        Self::new()
    }
}

enum TailType {
    JustFloat,
    Parameter,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_justfloat_frame() {
        let mut parser = UnifiedParser::new();
        let mut data = Vec::new();
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&2.0f32.to_le_bytes());
        data.extend_from_slice(&3.0f32.to_le_bytes());
        data.extend_from_slice(&JUST_FLOAT_TAIL);

        let frames = parser.feed_bytes(&data);
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            Frame::JustFloat(v) => assert_eq!(v, &vec![1.0, 2.0, 3.0]),
            _ => panic!("expected JustFloat"),
        }
    }

    #[test]
    fn test_multiple_justfloat_frames() {
        let mut parser = UnifiedParser::new();
        let mut data = Vec::new();
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&JUST_FLOAT_TAIL);
        data.extend_from_slice(&2.0f32.to_le_bytes());
        data.extend_from_slice(&3.0f32.to_le_bytes());
        data.extend_from_slice(&JUST_FLOAT_TAIL);

        let frames = parser.feed_bytes(&data);
        assert_eq!(frames.len(), 2);
        match &frames[0] {
            Frame::JustFloat(v) => assert_eq!(v, &vec![1.0]),
            _ => panic!("expected JustFloat"),
        }
        match &frames[1] {
            Frame::JustFloat(v) => assert_eq!(v, &vec![2.0, 3.0]),
            _ => panic!("expected JustFloat"),
        }
    }

    #[test]
    fn test_partial_feed() {
        let mut parser = UnifiedParser::new();
        let mut data = Vec::new();
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&2.0f32.to_le_bytes());

        let frames = parser.feed_bytes(&data[..4]);
        assert!(frames.is_empty());

        let frames = parser.feed_bytes(&data[4..]);
        assert!(frames.is_empty());

        let frames = parser.feed_bytes(&JUST_FLOAT_TAIL);
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            Frame::JustFloat(v) => assert_eq!(v, &vec![1.0, 2.0]),
            _ => panic!("expected JustFloat"),
        }
    }

    #[test]
    fn test_parameter_frame() {
        let mut parser = UnifiedParser::new();
        let mut data = Vec::new();
        let name1 = b"kp\0";
        data.extend_from_slice(name1);
        data.extend_from_slice(&1.5f32.to_le_bytes());
        let name2 = b"kd\0";
        data.extend_from_slice(name2);
        data.extend_from_slice(&0.5f32.to_le_bytes());
        data.extend_from_slice(&PARAMETER_TAIL);

        let frames = parser.feed_bytes(&data);
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            Frame::Parameter(entries) => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].name, "kp");
                assert_eq!(entries[0].value, 1.5);
                assert_eq!(entries[1].name, "kd");
                assert_eq!(entries[1].value, 0.5);
            }
            _ => panic!("expected Parameter"),
        }
    }

    #[test]
    fn test_mixed_frames() {
        let mut parser = UnifiedParser::new();
        let mut data = Vec::new();

        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&JUST_FLOAT_TAIL);

        let name1 = b"speed\0";
        data.extend_from_slice(name1);
        data.extend_from_slice(&100.0f32.to_le_bytes());
        data.extend_from_slice(&PARAMETER_TAIL);

        data.extend_from_slice(&2.0f32.to_le_bytes());
        data.extend_from_slice(&3.0f32.to_le_bytes());
        data.extend_from_slice(&JUST_FLOAT_TAIL);

        let frames = parser.feed_bytes(&data);
        assert_eq!(frames.len(), 3);
        match &frames[0] {
            Frame::JustFloat(v) => assert_eq!(v, &vec![1.0]),
            _ => panic!("expected JustFloat"),
        }
        match &frames[1] {
            Frame::Parameter(entries) => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].name, "speed");
                assert_eq!(entries[0].value, 100.0);
            }
            _ => panic!("expected Parameter"),
        }
        match &frames[2] {
            Frame::JustFloat(v) => assert_eq!(v, &vec![2.0, 3.0]),
            _ => panic!("expected JustFloat"),
        }
    }

    #[test]
    fn test_empty_feed() {
        let mut parser = UnifiedParser::new();
        let frames = parser.feed_bytes(&[]);
        assert!(frames.is_empty());
    }

    #[test]
    fn test_parameter_empty_name_skipped() {
        let mut parser = UnifiedParser::new();
        let mut data = Vec::new();
        data.push(0x00);
        let name = b"valid\0";
        data.extend_from_slice(name);
        data.extend_from_slice(&3.14f32.to_le_bytes());
        data.extend_from_slice(&PARAMETER_TAIL);

        let frames = parser.feed_bytes(&data);
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            Frame::Parameter(entries) => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].name, "valid");
            }
            _ => panic!("expected Parameter"),
        }
    }
}
