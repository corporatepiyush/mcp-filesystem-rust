use std::cmp::Ordering;
use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::path::{Component, Path, PathBuf};

// ──────────────────────────────────────────────
// PathTrie — Trie for efficient path prefix matching
// ──────────────────────────────────────────────

#[derive(Clone, Debug)]
struct PathTrieNode {
    children: Vec<(String, PathTrieNode)>,
    is_end: bool,
}

impl PathTrieNode {
    const fn new() -> Self {
        Self {
            children: Vec::new(),
            is_end: false,
        }
    }

    fn get_child(&self, component: &str) -> Option<&PathTrieNode> {
        self.children
            .iter()
            .find(|(name, _)| name == component)
            .map(|(_, node)| node)
    }

    fn get_or_insert_child(&mut self, component: &str) -> &mut PathTrieNode {
        let pos = self.children.iter().position(|(name, _)| name == component);
        match pos {
            Some(i) => &mut self.children[i].1,
            None => {
                self.children
                    .push((component.to_string(), PathTrieNode::new()));
                &mut self.children.last_mut().unwrap().1
            }
        }
    }
}

/// A trie-based data structure for efficient path prefix matching.
///
/// Allows fast checking of whether a given path is under any of the
/// inserted prefix paths.
#[derive(Clone, Debug)]
pub struct PathTrie {
    root: PathTrieNode,
}

impl PathTrie {
    pub const fn new() -> Self {
        Self {
            root: PathTrieNode::new(),
        }
    }

    /// Insert a path into the trie. Path components are split and
    /// stored as a chain. All intermediate nodes become valid prefix
    /// endpoints so that e.g. inserting `/a/b` also makes `/a` valid.
    pub fn insert(&mut self, path: &Path) {
        let mut current = &mut self.root;
        let comps: Vec<Component> = path.components().collect();
        if comps.is_empty() {
            current.is_end = true;
            return;
        }
        for component in &comps {
            match component {
                Component::RootDir => {
                    current = current.get_or_insert_child("/");
                }
                Component::Normal(name) => {
                    current = current.get_or_insert_child(&name.to_string_lossy());
                }
                Component::CurDir => {}
                Component::ParentDir => {}
                Component::Prefix(_) => {
                    current = current.get_or_insert_child(&component.as_os_str().to_string_lossy());
                }
            }
        }
        current.is_end = true;
    }

    /// Check if a path is under any prefix stored in the trie.
    /// Returns true if `path` has the same prefix as an inserted path.
    pub fn contains(&self, path: &Path) -> bool {
        let mut current = &self.root;
        let mut matched = current.is_end;

        let comps: Vec<Component> = path.components().collect();
        if comps.is_empty() {
            return matched;
        }
        for component in &comps {
            match component {
                Component::RootDir => match current.get_child("/") {
                    Some(child) => {
                        current = child;
                        if child.is_end {
                            matched = true;
                        }
                    }
                    None => return matched,
                },
                Component::Normal(name) => match current.get_child(&name.to_string_lossy()) {
                    Some(child) => {
                        current = child;
                        if child.is_end {
                            matched = true;
                        }
                    }
                    None => return matched,
                },
                Component::CurDir => {}
                Component::ParentDir => return false,
                Component::Prefix(_) => {
                    match current.get_child(&component.as_os_str().to_string_lossy()) {
                        Some(child) => {
                            current = child;
                            if child.is_end {
                                matched = true;
                            }
                        }
                        None => return matched,
                    }
                }
            }
        }

        matched
    }

    /// Find the longest matching prefix for a given path.
    pub fn longest_prefix(&self, path: &Path) -> Option<PathBuf> {
        let mut current = &self.root;
        let mut result = PathBuf::new();
        let mut last_match = if current.is_end {
            Some(result.clone())
        } else {
            None
        };

        for component in path.components() {
            match component {
                Component::RootDir => {
                    if let Some(child) = current.get_child("/") {
                        result.push("/");
                        current = child;
                        if child.is_end {
                            last_match = Some(result.clone());
                        }
                    } else {
                        break;
                    }
                }
                Component::Normal(name) => {
                    let name_str = name.to_string_lossy();
                    if let Some(child) = current.get_child(&name_str) {
                        result.push(name_str.as_ref());
                        current = child;
                        if child.is_end {
                            last_match = Some(result.clone());
                        }
                    } else {
                        break;
                    }
                }
                Component::CurDir => {}
                Component::ParentDir => break,
                Component::Prefix(_) => {
                    let name_str = component.as_os_str().to_string_lossy();
                    if let Some(child) = current.get_child(&name_str) {
                        result.push(name_str.as_ref());
                        current = child;
                        if child.is_end {
                            last_match = Some(result.clone());
                        }
                    } else {
                        break;
                    }
                }
            }
        }

        last_match
    }

    /// Insert multiple paths into the trie.
    pub fn extend(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            self.insert(&path);
        }
    }
}

impl Default for PathTrie {
    fn default() -> Self {
        Self::new()
    }
}

// ──────────────────────────────────────────────
// RingBuffer — Fixed-size circular buffer
// ──────────────────────────────────────────────

/// A fixed-capacity ring buffer (circular buffer).
/// When full, new items overwrite the oldest items.
#[derive(Clone, Debug)]
pub struct RingBuffer<T> {
    buffer: Vec<Option<T>>,
    head: usize,
    size: usize,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        let mut buffer = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            buffer.push(None);
        }
        Self {
            buffer,
            head: 0,
            size: 0,
            capacity,
        }
    }

    pub fn push(&mut self, item: T) {
        if self.size == self.capacity {
            self.buffer[self.head] = Some(item);
            self.head = (self.head + 1) % self.capacity;
        } else {
            let idx = (self.head + self.size) % self.capacity;
            self.buffer[idx] = Some(item);
            self.size += 1;
        }
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.size == 0 {
            return None;
        }
        let item = self.buffer[self.head].take();
        self.head = (self.head + 1) % self.capacity;
        self.size -= 1;
        item
    }

    pub const fn len(&self) -> usize {
        self.size
    }

    pub const fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub const fn is_full(&self) -> bool {
        self.size == self.capacity
    }

    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Collect all items in order (oldest first) into a Vec of references.
    pub fn to_vec(&self) -> Vec<&T> {
        let mut result = Vec::with_capacity(self.size);
        for i in 0..self.size {
            let idx = (self.head + i) % self.capacity;
            if let Some(ref item) = self.buffer[idx] {
                result.push(item);
            }
        }
        result
    }

    /// Consume the buffer and return items in order (oldest first).
    pub fn into_vec(mut self) -> Vec<T> {
        let mut result = Vec::with_capacity(self.size);
        while let Some(item) = self.pop() {
            result.push(item);
        }
        result
    }

    pub const fn iter(&self) -> RingBufferIter<'_, T> {
        RingBufferIter {
            buffer: self,
            index: 0,
        }
    }
}

impl<'a, T> IntoIterator for &'a RingBuffer<T> {
    type Item = &'a T;
    type IntoIter = RingBufferIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct RingBufferIter<'a, T> {
    buffer: &'a RingBuffer<T>,
    index: usize,
}

impl<'a, T> Iterator for RingBufferIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.buffer.size {
            return None;
        }
        let idx = (self.buffer.head + self.index) % self.buffer.capacity;
        self.index += 1;
        self.buffer.buffer[idx].as_ref()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.buffer.size.saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

// ──────────────────────────────────────────────
// LruCache — Simple LRU cache
// ──────────────────────────────────────────────

/// A simple LRU (Least Recently Used) cache with fixed capacity.
/// Uses O(n) linear scan; suitable for small caches (≤256 entries).
#[derive(Debug, Clone)]
pub struct LruCache<K, V> {
    entries: VecDeque<(K, V)>,
    capacity: usize,
}

impl<K: Eq, V> LruCache<K, V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        let pos = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(pos).unwrap();
        self.entries.push_front(entry);
        Some(&self.entries.front().unwrap().1)
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let pos = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(pos).unwrap();
        self.entries.push_front(entry);
        Some(&mut self.entries.front_mut().unwrap().1)
    }

    pub fn peek(&self, key: &K) -> Option<&V> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn put(&mut self, key: K, value: V) {
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == &key) {
            self.entries.remove(pos);
        } else if self.entries.len() >= self.capacity {
            self.entries.pop_back();
        }
        self.entries.push_front((key, value));
    }

    pub fn contains(&self, key: &K) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let pos = self.entries.iter().position(|(k, _)| k == key)?;
        let (_, v) = self.entries.remove(pos).unwrap();
        Some(v)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn iter(&self) -> impl Iterator<Item = &(K, V)> {
        self.entries.iter()
    }
}

// ──────────────────────────────────────────────
// SortedVec — Insert-sorted vector
// ──────────────────────────────────────────────

/// A vector that maintains elements in sorted order.
/// Uses binary search for insertion (O(log n)) and
/// linear shift (O(n)). Best for small to medium collections.
#[derive(Debug, Clone)]
pub struct SortedVec<T: Ord> {
    inner: Vec<T>,
}

impl<T: Ord> SortedVec<T> {
    pub const fn new() -> Self {
        Self { inner: Vec::new() }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Vec::with_capacity(capacity),
        }
    }

    /// Insert an element in sorted position. Returns the insertion index.
    pub fn insert(&mut self, item: T) -> usize {
        let pos = self.inner.binary_search(&item).unwrap_or_else(|e| e);
        self.inner.insert(pos, item);
        pos
    }

    /// Remove an element. Returns true if found and removed.
    pub fn remove(&mut self, item: &T) -> bool {
        self.inner.binary_search(item).is_ok_and(|pos| {
            self.inner.remove(pos);
            true
        })
    }

    pub fn contains(&self, item: &T) -> bool {
        self.inner.binary_search(item).is_ok()
    }

    pub fn binary_search(&self, item: &T) -> Result<usize, usize> {
        self.inner.binary_search(item)
    }

    pub fn into_inner(self) -> Vec<T> {
        self.inner
    }

    pub fn as_slice(&self) -> &[T] {
        &self.inner
    }

    pub const fn len(&self) -> usize {
        self.inner.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.inner.get(index)
    }

    pub fn pop(&mut self) -> Option<T> {
        self.inner.pop()
    }

    pub fn drain(&mut self, range: std::ops::Range<usize>) -> impl Iterator<Item = T> + '_ {
        self.inner.drain(range)
    }
}

impl<T: Ord> Default for SortedVec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Ord> From<Vec<T>> for SortedVec<T> {
    fn from(mut vec: Vec<T>) -> Self {
        vec.sort();
        Self { inner: vec }
    }
}

impl<T: Ord> FromIterator<T> for SortedVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut inner: Vec<T> = iter.into_iter().collect();
        inner.sort();
        Self { inner }
    }
}

impl<T: Ord> IntoIterator for SortedVec<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<'a, T: Ord> IntoIterator for &'a SortedVec<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}

// ──────────────────────────────────────────────
// BloomFilter — Space-efficient set membership
// ──────────────────────────────────────────────

/// A simple Bloom filter for probabilistic set membership checks.
/// Supports insert and contains with configurable false positive rate.
pub struct BloomFilter {
    bits: Vec<u64>,
    num_hashes: usize,
    size: usize,
    inserted: usize,
}

impl BloomFilter {
    pub fn new(expected_items: usize, false_positive_rate: f64) -> Self {
        let size = optimal_bit_count(expected_items, false_positive_rate).max(1);
        let num_hashes = optimal_hash_count(expected_items, size).max(1);
        let num_u64s = size.div_ceil(64);
        Self {
            bits: vec![0u64; num_u64s],
            num_hashes,
            size,
            inserted: 0,
        }
    }

    pub fn insert(&mut self, item: &[u8]) {
        let (h1, h2) = hash_pair(item);
        for i in 0..self.num_hashes {
            let bit = (h1.wrapping_add((i as u64).wrapping_mul(h2))) % self.size as u64;
            let idx = bit as usize / 64;
            let off = bit as usize % 64;
            self.bits[idx] |= 1u64 << off;
        }
        self.inserted += 1;
    }

    pub fn contains(&self, item: &[u8]) -> bool {
        let (h1, h2) = hash_pair(item);
        for i in 0..self.num_hashes {
            let bit = (h1.wrapping_add((i as u64).wrapping_mul(h2))) % self.size as u64;
            let idx = bit as usize / 64;
            let off = bit as usize % 64;
            if self.bits[idx] & (1u64 << off) == 0 {
                return false;
            }
        }
        true
    }

    pub const fn len(&self) -> usize {
        self.inserted
    }

    pub const fn is_empty(&self) -> bool {
        self.inserted == 0
    }

    pub fn clear(&mut self) {
        for word in &mut self.bits {
            *word = 0;
        }
        self.inserted = 0;
    }
}

fn hash_pair(data: &[u8]) -> (u64, u64) {
    let mut h1 = DefaultHasher::new();
    h1.write(data);
    let hash1 = h1.finish();

    let mut h2 = DefaultHasher::new();
    h2.write(data);
    h2.write(b"\x01second");
    let hash2 = h2.finish();

    (hash1, hash2)
}

#[allow(clippy::cast_precision_loss)]
fn optimal_bit_count(n: usize, p: f64) -> usize {
    if n == 0 {
        return 1;
    }
    let m = -(n as f64) * p.ln() / (std::f64::consts::LN_2.powi(2));
    m.ceil() as usize
}

#[allow(clippy::cast_precision_loss)]
fn optimal_hash_count(n: usize, m: usize) -> usize {
    if n == 0 || m == 0 {
        return 1;
    }
    let k = (m as f64 / n as f64) * std::f64::consts::LN_2;
    (k.ceil() as usize).max(1)
}

// ──────────────────────────────────────────────
// Algorithms
// ──────────────────────────────────────────────

/// Compute the Levenshtein distance between two strings.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_len = a.chars().count();
    let b_len = b.chars().count();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row = vec![0usize; b_len + 1];

    for (i, a_char) in a.chars().enumerate() {
        curr_row[0] = i + 1;
        for (j, b_char) in b.chars().enumerate() {
            let cost = if a_char == b_char { 0 } else { 1 };
            curr_row[j + 1] = std::cmp::min(
                std::cmp::min(curr_row[j] + 1, prev_row[j + 1] + 1),
                prev_row[j] + cost,
            );
        }
        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

/// Compute the Jaro-Winkler similarity between two strings.
/// Returns a value between 0.0 and 1.0.
#[allow(clippy::cast_precision_loss)]
pub fn jaro_winkler_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    let match_distance = (a_len.max(b_len) / 2).saturating_sub(1);

    let mut a_matched = vec![false; a_len];
    let mut b_matched = vec![false; b_len];
    let mut matches = 0usize;
    let mut transpositions = 0usize;

    for i in 0..a_len {
        let start = i.saturating_sub(match_distance);
        let end = (i + match_distance + 1).min(b_len);
        for j in start..end {
            if b_matched[j] {
                continue;
            }
            if a_chars[i] != b_chars[j] {
                continue;
            }
            a_matched[i] = true;
            b_matched[j] = true;
            matches += 1;
            break;
        }
    }

    if matches == 0 {
        return 0.0;
    }

    let mut k = 0usize;
    for i in 0..a_len {
        if !a_matched[i] {
            continue;
        }
        while !b_matched[k] {
            k += 1;
        }
        if a_chars[i] != b_chars[k] {
            transpositions += 1;
        }
        k += 1;
    }

    let jaro = (matches as f64 / a_len as f64
        + matches as f64 / b_len as f64
        + (matches as f64 - transpositions as f64 / 2.0) / matches as f64)
        / 3.0;

    let mut prefix = 0usize;
    let max_prefix = a_len.min(b_len).min(4);
    for i in 0..max_prefix {
        if a_chars[i] == b_chars[i] {
            prefix += 1;
        } else {
            break;
        }
    }

    jaro + (prefix as f64 * 0.1 * (1.0 - jaro))
}

/// Fuzzy match a pattern against a string.
/// Returns true for substring matches, or close Levenshtein matches.
pub fn fuzzy_match(pattern: &str, text: &str, max_distance: Option<usize>) -> bool {
    let pattern_lower = pattern.to_lowercase();
    let text_lower = text.to_lowercase();

    if text_lower.contains(&pattern_lower) {
        return true;
    }

    if let Some(max_dist) = max_distance {
        let len_diff = text_lower.len().abs_diff(pattern_lower.len());
        if len_diff <= max_dist {
            let dist = levenshtein_distance(&text_lower, &pattern_lower);
            if dist <= max_dist {
                return true;
            }
        }
    }

    false
}

/// Merge two sorted slices into a new sorted Vec.
pub fn merge_sorted<T: Ord + Clone>(a: &[T], b: &[T]) -> Vec<T> {
    let mut result = Vec::with_capacity(a.len() + b.len());
    let mut i = 0;
    let mut j = 0;

    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            Ordering::Less | Ordering::Equal => {
                result.push(a[i].clone());
                i += 1;
            }
            Ordering::Greater => {
                result.push(b[j].clone());
                j += 1;
            }
        }
    }

    result.extend_from_slice(&a[i..]);
    result.extend_from_slice(&b[j..]);

    result
}

/// Binary search on a slice by a projection function.
pub fn binary_search_by_projection<T, U: Ord>(
    slice: &[T],
    target: &U,
    projection: impl Fn(&T) -> &U,
) -> Result<usize, usize> {
    slice.binary_search_by(|item| projection(item).cmp(target))
}

// ──────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_path_trie_basic() {
        let mut trie = PathTrie::new();
        trie.insert(Path::new("/home/user/projects"));

        assert!(trie.contains(Path::new("/home/user/projects")));
        assert!(trie.contains(Path::new("/home/user/projects/src")));
        assert!(trie.contains(Path::new("/home/user/projects/src/main.rs")));
        assert!(!trie.contains(Path::new("/home/user")));
        assert!(!trie.contains(Path::new("/etc")));
    }

    #[test]
    fn test_path_trie_multiple() {
        let mut trie = PathTrie::new();
        trie.insert(Path::new("/home/user/projects"));
        trie.insert(Path::new("/var/log"));

        assert!(trie.contains(Path::new("/home/user/projects/mcp")));
        assert!(trie.contains(Path::new("/var/log/syslog")));
        assert!(!trie.contains(Path::new("/home/user")));
        assert!(!trie.contains(Path::new("/var")));
    }

    #[test]
    fn test_ring_buffer() {
        let mut buf = RingBuffer::new(3);
        assert!(buf.is_empty());

        buf.push(1);
        buf.push(2);
        buf.push(3);
        assert!(buf.is_full());
        assert_eq!(buf.to_vec(), vec![&1, &2, &3]);

        buf.push(4);
        assert_eq!(buf.to_vec(), vec![&2, &3, &4]);

        assert_eq!(buf.pop(), Some(2));
        assert_eq!(buf.pop(), Some(3));
        assert_eq!(buf.pop(), Some(4));
        assert!(buf.is_empty());
    }

    #[test]
    fn test_ring_buffer_iterator() {
        let mut buf = RingBuffer::new(3);
        buf.push(10);
        buf.push(20);
        buf.push(30);

        let collected: Vec<&i32> = buf.iter().collect();
        assert_eq!(collected, vec![&10, &20, &30]);
    }

    #[test]
    fn test_lru_cache() {
        let mut cache = LruCache::new(3);
        cache.put("a", 1);
        cache.put("b", 2);
        cache.put("c", 3);
        assert_eq!(cache.len(), 3);

        let _ = cache.get(&"a");
        cache.put("d", 4);
        assert!(!cache.contains(&"b"));
        assert!(cache.contains(&"a"));
        assert!(cache.contains(&"c"));
        assert!(cache.contains(&"d"));

        assert_eq!(cache.remove(&"a"), Some(1));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_sorted_vec() {
        let mut sv = SortedVec::new();
        sv.insert(3);
        sv.insert(1);
        sv.insert(2);
        assert_eq!(sv.as_slice(), &[1, 2, 3]);

        assert!(sv.contains(&2));
        assert!(!sv.contains(&4));

        assert!(sv.remove(&2));
        assert!(!sv.contains(&2));

        let inner: Vec<i32> = sv.into_inner();
        assert_eq!(inner, vec![1, 3]);
    }

    #[test]
    fn test_sorted_vec_from_vec() {
        let sv: SortedVec<i32> = SortedVec::from(vec![3, 1, 4, 1, 5, 9]);
        assert_eq!(sv.as_slice(), &[1, 1, 3, 4, 5, 9]);
    }

    #[test]
    fn test_bloom_filter() {
        let mut bf = BloomFilter::new(100, 0.01);
        bf.insert(b"hello");
        bf.insert(b"world");

        assert!(bf.contains(b"hello"));
        assert!(bf.contains(b"world"));
    }

    #[test]
    fn test_levenshtein() {
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("hello", "hello"), 0);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
    }

    #[test]
    fn test_jaro_winkler() {
        let sim = jaro_winkler_similarity("martha", "marhta");
        assert!((sim - 0.9611).abs() < 0.01);

        let exact = jaro_winkler_similarity("hello", "hello");
        assert!((exact - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_fuzzy_match() {
        assert!(fuzzy_match("hello", "hello world", None));
        assert!(fuzzy_match("HELLO", "hello world", None));
        assert!(fuzzy_match("helo", "hello", Some(1)));
        assert!(!fuzzy_match("xyzzy", "hello world", Some(2)));
    }

    #[test]
    fn test_merge_sorted() {
        let a = vec![1, 3, 5];
        let b = vec![2, 4, 6];
        let merged = merge_sorted(&a, &b);
        assert_eq!(merged, vec![1, 2, 3, 4, 5, 6]);

        let empty: Vec<i32> = vec![];
        let merged = merge_sorted(&a, &empty);
        assert_eq!(merged, vec![1, 3, 5]);
    }

    #[test]
    fn test_binary_search_by_projection() {
        let items = vec![(1, "a"), (3, "b"), (5, "c")];
        let result = binary_search_by_projection(&items, &3, |(k, _)| k);
        assert_eq!(result, Ok(1));

        let result = binary_search_by_projection(&items, &4, |(k, _)| k);
        assert_eq!(result, Err(2));
    }

    #[test]
    fn test_longest_prefix() {
        let mut trie = PathTrie::new();
        trie.insert(Path::new("/home/user/projects"));
        trie.insert(Path::new("/home/user"));

        let lp = trie.longest_prefix(Path::new("/home/user/projects/mcp/src"));
        assert!(lp.is_some());
        assert_eq!(lp.unwrap(), PathBuf::from("/home/user/projects"));
    }
}
