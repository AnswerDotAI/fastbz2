const HASH_SIZE: usize = 1 << 16;
const NONE: u32 = u32::MAX;

#[inline]
pub(crate) fn match_length(bytes: &[u8], left: usize, right: usize, limit: usize) -> usize {
    let mut length = 0;
    while length + 8 <= limit {
        let a = u64::from_le_bytes(bytes[left + length..left + length + 8].try_into().unwrap());
        let b = u64::from_le_bytes(bytes[right + length..right + length + 8].try_into().unwrap());
        let different = a ^ b;
        if different != 0 { return length + (different.trailing_zeros() as usize / 8); }
        length += 8;
    }
    while length < limit && bytes[left + length] == bytes[right + length] { length += 1; }
    length
}

pub(crate) struct LatestMatch { head: Vec<u32>, window: usize }

impl LatestMatch {
    pub fn new(window: usize) -> Self { Self { head: vec![NONE; HASH_SIZE], window } }

    #[inline]
    fn hash(bytes: &[u8], position: usize) -> usize {
        let value = u32::from_le_bytes(bytes[position..position + 4].try_into().unwrap());
        ((value.wrapping_mul(0x9e37_79b1)) >> 16) as usize
    }

    #[inline]
    pub fn insert_and_find(&mut self, bytes: &[u8], position: usize) -> Option<usize> {
        if position + 4 > bytes.len() { return None; }
        let slot = Self::hash(bytes, position);
        let candidate = self.head[slot];
        self.head[slot] = position as u32;
        let candidate = (candidate != NONE).then_some(candidate as usize)?;
        (position - candidate <= self.window && bytes[candidate..candidate + 4] == bytes[position..position + 4]).then_some(candidate)
    }

    #[inline]
    pub fn insert(&mut self, bytes: &[u8], position: usize) { if position + 4 <= bytes.len() { self.head[Self::hash(bytes, position)] = position as u32; } }
}

pub(crate) struct HashChain { head: Vec<u32>, previous: Option<Vec<u32>>, window: usize }

impl HashChain {
    pub fn new(input_len: usize, window: usize, max_chain: usize) -> Self {
        assert!(input_len < u32::MAX as usize);
        Self { head: vec![NONE; HASH_SIZE], previous: (max_chain > 1).then(|| vec![NONE; input_len]), window }
    }

    fn hash(bytes: &[u8], position: usize) -> usize {
        let value = u32::from(bytes[position]) << 16 | u32::from(bytes[position + 1]) << 8 | u32::from(bytes[position + 2]);
        ((value.wrapping_mul(0x1e35_a7bd)) >> 16) as usize
    }

    pub fn insert(&mut self, bytes: &[u8], position: usize) {
        if position + 2 >= bytes.len() { return; }
        let slot = Self::hash(bytes, position);
        if let Some(previous) = &mut self.previous { previous[position] = self.head[slot]; }
        self.head[slot] = position as u32;
    }

    pub fn best_match(&self, bytes: &[u8], position: usize, max_length: usize, min_length: usize, max_chain: usize) -> (usize, usize) {
        if position + min_length > bytes.len() || min_length < 3 { return (0, 0); }
        let mut candidate = self.head[Self::hash(bytes, position)];
        let minimum = position.saturating_sub(self.window);
        let limit = (bytes.len() - position).min(max_length);
        let mut best_length = min_length - 1;
        let mut best_distance = 0;
        let mut searched = 0;
        while candidate != NONE && candidate as usize >= minimum && searched < max_chain {
            let candidate_position = candidate as usize;
            let distance = position - candidate_position;
            if bytes[candidate_position] == bytes[position] && bytes.get(candidate_position + best_length) == bytes.get(position + best_length) {
                let length = match_length(bytes, candidate_position, position, limit);
                if length > best_length {
                    best_length = length;
                    best_distance = distance;
                    if length == limit { break; }
                }
            }
            searched += 1;
            if searched >= max_chain { break; }
            candidate = self.previous.as_ref().map_or(NONE, |previous| previous[candidate_position]);
        }
        if best_length >= min_length { (best_length, best_distance) } else { (0, 0) }
    }
}
