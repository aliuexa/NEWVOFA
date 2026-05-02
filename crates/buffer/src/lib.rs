use std::collections::VecDeque;

pub struct RingBuffer<T> {
    buffer: VecDeque<T>,
    capacity: usize,
}

impl<T: Clone> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "RingBuffer capacity must be greater than 0");
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, value: T) {
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(value);
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.buffer.iter()
    }

    pub fn as_slices(&self) -> (&[T], &[T]) {
        self.buffer.as_slices()
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

pub struct DataStore {
    channels: Vec<RingBuffer<f32>>,
    sample_count: u64,
}

impl DataStore {
    pub fn new(num_channels: usize, max_points_per_channel: usize) -> Self {
        let channels = (0..num_channels)
            .map(|_| RingBuffer::new(max_points_per_channel))
            .collect();
        Self {
            channels,
            sample_count: 0,
        }
    }

    pub fn push_frame(&mut self, frame: &[f32]) {
        let num_channels = self.channels.len();
        for (i, &value) in frame.iter().enumerate() {
            if i < num_channels {
                self.channels[i].push(value);
            }
        }
        self.sample_count += 1;
    }

    pub fn channel_data(&self, channel: usize) -> Option<&RingBuffer<f32>> {
        self.channels.get(channel)
    }

    pub fn num_channels(&self) -> usize {
        self.channels.len()
    }

    pub fn sample_count(&self) -> u64 {
        self.sample_count
    }

    pub fn clear(&mut self) {
        for channel in &mut self.channels {
            channel.clear();
        }
        self.sample_count = 0;
    }

    pub fn resize_channels(&mut self, num_channels: usize, max_points: usize) {
        let old_len = self.channels.len();
        if num_channels > old_len {
            for _ in old_len..num_channels {
                self.channels.push(RingBuffer::new(max_points));
            }
        } else if num_channels < old_len {
            self.channels.truncate(num_channels);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_push() {
        let mut rb = RingBuffer::new(3);
        rb.push(1.0);
        rb.push(2.0);
        rb.push(3.0);
        rb.push(4.0);
        let values: Vec<f32> = rb.iter().copied().collect();
        assert_eq!(values, vec![2.0, 3.0, 4.0]);
    }

    #[test]
    fn test_data_store_push_frame() {
        let mut store = DataStore::new(3, 100);
        store.push_frame(&[1.0, 2.0, 3.0]);
        store.push_frame(&[4.0, 5.0, 6.0]);

        let ch0: Vec<f32> = store.channel_data(0).unwrap().iter().copied().collect();
        assert_eq!(ch0, vec![1.0, 4.0]);
    }

    #[test]
    fn test_data_store_extra_channels_in_frame() {
        let mut store = DataStore::new(2, 100);
        store.push_frame(&[1.0, 2.0, 3.0, 4.0]);
        let ch0: Vec<f32> = store.channel_data(0).unwrap().iter().copied().collect();
        let ch1: Vec<f32> = store.channel_data(1).unwrap().iter().copied().collect();
        assert_eq!(ch0, vec![1.0]);
        assert_eq!(ch1, vec![2.0]);
    }
}
