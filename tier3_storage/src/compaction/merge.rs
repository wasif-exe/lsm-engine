use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::path::PathBuf;

use crate::sstable::streaming::StreamingSSTableIterator;
use crate::sstable::writer::SSTableWriter;

struct MergeElement {
    key: Vec<u8>,
    val: Option<Vec<u8>>,
    seq: u64,
    source_idx: usize,
}

impl PartialEq for MergeElement {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.seq == other.seq
    }
}

impl Eq for MergeElement {}

impl PartialOrd for MergeElement {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// Min-heap ordering: lowest key first; on tie, highest sequence number first.
impl Ord for MergeElement {
    fn cmp(&self, other: &Self) -> Ordering {
        other.key.cmp(&self.key)
            .then_with(|| self.seq.cmp(&other.seq))
    }
}

pub struct StreamingKWayMerge;

impl StreamingKWayMerge {
    pub fn merge(
        input_paths: &[PathBuf],
        output_path: &PathBuf,
        is_bottom_level: bool,
    ) -> std::io::Result<u64> {
        let mut iters: Vec<StreamingSSTableIterator> = Vec::new();
        for p in input_paths {
            iters.push(StreamingSSTableIterator::open(p)?);
        }

        let mut heap: BinaryHeap<MergeElement> = BinaryHeap::new();

        for (idx, iter) in iters.iter_mut().enumerate() {
            if let Some((key, val, seq)) = iter.next_kv() {
                heap.push(MergeElement {
                    key,
                    val,
                    seq,
                    source_idx: idx,
                });
            }
        }

        let mut writer = SSTableWriter::create(output_path)?;
        let mut last_emitted_key: Option<Vec<u8>> = None;

        while let Some(top) = heap.pop() {
            let src = top.source_idx;
            let current_key = top.key;
            let current_val = top.val;
            let current_seq = top.seq;

            // Fetch next item from the same iterator
            if let Some((k, v, s)) = iters[src].next_kv() {
                heap.push(MergeElement {
                    key: k,
                    val: v,
                    seq: s,
                    source_idx: src,
                });
            }

            // Deduplication: if key matches previously emitted key, discard (older seq num).
            if let Some(ref prev) = last_emitted_key {
                if prev == &current_key {
                    continue;
                }
            }

            // At bottom level, drop tombstones permanently (Space Reclamation)
            if is_bottom_level && current_val.is_none() {
                last_emitted_key = Some(current_key);
                continue;
            }

            writer.append(&current_key, current_val.as_deref(), current_seq)?;
            last_emitted_key = Some(current_key);
        }

        writer.finish()
    }
}