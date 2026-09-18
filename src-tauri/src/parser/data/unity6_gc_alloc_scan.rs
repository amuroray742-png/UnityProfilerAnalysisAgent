//! Unity 6 GC.Alloc metadata 盲扫
//!
//! 思路：在 frame body 中每个 0..7 字节对齐偏移，扫 8 字节步进的
//! `(sampleIndex i32, bytes u32)` 对，过滤：
//! - `0 <= sampleIndex < 1_000_000`（单线程样本上限）
//! - `1 <= bytes <= 1 GB`（合理 GC.Alloc 大小）
//!
//! 报告最长的连续 run（最可能是 GC.Alloc metadata 段）。

#[derive(Debug, Clone, Copy)]
pub struct CandidatePair {
    pub body_offset: usize,
    pub sample_index: i32,
    pub alloc_bytes: u32,
}

pub fn scan(body: &[u8]) -> Vec<CandidatePair> {
    const MAX_SAMPLE_INDEX: i32 = 1_000_000;
    const MIN_BYTES: u32 = 1;
    const MAX_BYTES: u32 = 1 << 30; // 1 GB

    let mut out = Vec::new();
    // 8 字节对齐循环；一次 unsafe u64 读 → (i32, u32) 位拆。
    // x86_64 上 unaligned 8-byte load 是 native 指令（movq），
    // 比 from_le_bytes([..]) 少一次临时数组。
    let chunks = body.chunks_exact(8);
    let _remainder = chunks.remainder();
    for (i, chunk) in chunks.enumerate() {
        let pair: u64 = unsafe {
            // SAFETY: chunks_exact(8) 保证每个 chunk 长度 = 8，地址 8-byte 加载是 valid
            std::ptr::read_unaligned(chunk.as_ptr() as *const u64)
        };
        let sample_index = pair as i32;
        let bytes = (pair >> 32) as u32;
        if (0..=MAX_SAMPLE_INDEX).contains(&sample_index)
            && (MIN_BYTES..=MAX_BYTES).contains(&bytes)
        {
            out.push(CandidatePair {
                body_offset: i * 8,
                sample_index,
                alloc_bytes: bytes,
            });
        }
    }
    out
}

/// 找最长的连续 run（同一对齐偏移、严格递增 sampleIndex、连续字节位置）。
pub fn longest_run(pairs: &[CandidatePair]) -> &[CandidatePair] {
    if pairs.is_empty() {
        return &[];
    }
    let mut best_start = 0usize;
    let mut best_len = 0usize;
    let mut cur_start = 0usize;
    let mut cur_len = 1usize;
    for i in 1..pairs.len() {
        let prev = &pairs[i - 1];
        let cur = &pairs[i];
        let contiguous = cur.body_offset == prev.body_offset + 8;
        let ascending = cur.sample_index > prev.sample_index;
        if contiguous && ascending {
            cur_len += 1;
        } else {
            if cur_len > best_len {
                best_start = cur_start;
                best_len = cur_len;
            }
            cur_start = i;
            cur_len = 1;
        }
    }
    if cur_len > best_len {
        best_start = cur_start;
        best_len = cur_len;
    }
    &pairs[best_start..best_start + best_len]
}

/// 扫所有 8 字节对齐偏移（0..8），返回各对齐的最长 run。
#[derive(Debug)]
pub struct AlignmentRun {
    pub alignment: usize,
    pub pairs: Vec<CandidatePair>,
}

pub fn scan_all_alignments(body: &[u8]) -> Vec<AlignmentRun> {
    let mut out = Vec::new();
    for align in 0..8usize {
        if align >= body.len() {
            break;
        }
        let aligned_body = &body[align..];
        let pairs = scan(aligned_body);
        let run: Vec<CandidatePair> = longest_run(&pairs).to_vec();
        out.push(AlignmentRun {
            alignment: align,
            pairs: run,
        });
    }
    out
}

/// 提取 Unity 6 GC.Alloc metadata：找最强 run 并返回 (total_alloc_bytes, sample_pairs)
/// - 强信号：alignment=0 + max_consecutive_+1 ≥ STRONG_THRESHOLD（足够可信，提前退出）
/// - 弱信号：max_consecutive_+1 ≥ WEAK_THRESHOLD（alignment=0 没有强信号时才纳入）
#[derive(Debug)]
pub struct GcAllocExtraction {
    pub alignment: usize,
    pub run_length: usize,
    pub max_consecutive_plus1: usize,
    pub total_alloc_bytes: u64,
    pub pairs: Vec<CandidatePair>,
}

const STRONG_THRESHOLD: usize = 10;
const WEAK_THRESHOLD: usize = 3;

fn compute_max_consec(run: &[CandidatePair]) -> usize {
    let mut max_consec = 1usize;
    let mut cur_consec = 1usize;
    for w in run.windows(2) {
        if w[1].body_offset == w[0].body_offset + 8
            && w[1].sample_index == w[0].sample_index + 1
        {
            cur_consec += 1;
            if cur_consec > max_consec {
                max_consec = cur_consec;
            }
        } else {
            cur_consec = 1;
        }
    }
    max_consec
}

pub fn extract(body: &[u8]) -> Option<GcAllocExtraction> {
    // 第一阶段：只扫 alignment=0
    if !body.is_empty() {
        let pairs0 = scan(&body[0..]);
        let run0 = longest_run(&pairs0);
        let max_consec0 = compute_max_consec(run0);
        if max_consec0 >= STRONG_THRESHOLD {
            let total: u64 = run0.iter().map(|p| p.alloc_bytes as u64).sum();
            return Some(GcAllocExtraction {
                alignment: 0,
                run_length: run0.len(),
                max_consecutive_plus1: max_consec0,
                total_alloc_bytes: total,
                pairs: run0.to_vec(),
            });
        }
    }

    // 第二阶段：alignment=0 没有强信号，扫全部 alignment 的弱信号并累加
    let mut total_bytes: u64 = 0;
    let mut total_pairs: usize = 0;
    let mut max_consec_overall: usize = 0;
    let mut all_pairs: Vec<CandidatePair> = vec![];

    let runs = scan_all_alignments(body);
    for r in runs {
        if r.pairs.len() < WEAK_THRESHOLD {
            continue;
        }
        let max_consec = compute_max_consec(&r.pairs);
        if max_consec >= WEAK_THRESHOLD {
            let seg_total: u64 = r.pairs.iter().map(|p| p.alloc_bytes as u64).sum();
            total_bytes += seg_total;
            total_pairs += r.pairs.len();
            if max_consec > max_consec_overall {
                max_consec_overall = max_consec;
            }
            all_pairs.extend_from_slice(&r.pairs);
        }
    }

    if total_pairs == 0 {
        return None;
    }
    Some(GcAllocExtraction {
        alignment: 0,
        run_length: total_pairs,
        max_consecutive_plus1: max_consec_overall,
        total_alloc_bytes: total_bytes,
        pairs: all_pairs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_candidate_pairs() {
        let mut body = vec![0u8; 64];
        // sampleIndex=5, allocBytes=1024 at offset 8
        body[8..12].copy_from_slice(&5i32.to_le_bytes());
        body[12..16].copy_from_slice(&1024u32.to_le_bytes());
        // sampleIndex=10, allocBytes=2048 at offset 16
        body[16..20].copy_from_slice(&10i32.to_le_bytes());
        body[20..24].copy_from_slice(&2048u32.to_le_bytes());
        // junk: negative sampleIndex
        body[24..28].copy_from_slice(&(-1i32).to_le_bytes());
        body[28..32].copy_from_slice(&1024u32.to_le_bytes());
        let pairs = scan(&body);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].sample_index, 5);
        assert_eq!(pairs[0].alloc_bytes, 1024);
        assert_eq!(pairs[1].sample_index, 10);
    }

    #[test]
    fn finds_longest_run() {
        let mut body = vec![0u8; 128];
        // contiguous run: 5, 6, 7 at offsets 0, 8, 16
        for (i, idx) in [5i32, 6, 7].iter().enumerate() {
            let off = i * 8;
            body[off..off + 4].copy_from_slice(&idx.to_le_bytes());
            body[off + 4..off + 8].copy_from_slice(&1024u32.to_le_bytes());
        }
        // gap, then another run of length 4
        body[64..68].copy_from_slice(&20i32.to_le_bytes());
        body[68..72].copy_from_slice(&2048u32.to_le_bytes());
        body[72..76].copy_from_slice(&21i32.to_le_bytes());
        body[76..80].copy_from_slice(&2048u32.to_le_bytes());
        body[80..84].copy_from_slice(&22i32.to_le_bytes());
        body[84..88].copy_from_slice(&2048u32.to_le_bytes());
        body[88..92].copy_from_slice(&23i32.to_le_bytes());
        body[92..96].copy_from_slice(&2048u32.to_le_bytes());

        let pairs = scan(&body);
        let run = longest_run(&pairs);
        assert_eq!(run.len(), 4);
        assert_eq!(run[0].sample_index, 20);
        assert_eq!(run[3].sample_index, 23);
    }
}