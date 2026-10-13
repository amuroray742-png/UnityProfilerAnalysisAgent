//! Unity 6000.x 内存计数器通用扫描器
//!
//! 与 Unity 2022.3 不同：Unity 6 的 Memory counter schema 没有公开文档。
//! 改为「扫描整个 body 找连续升序 counterId 段」，提取每条 (id, type, value)
//! 三元组，不假设固定 record 数。
//!
//! 用户实测：frame body offset 72672 (441680 字节 body 的 ~16%) 起，至少
//! 22 条 record（1 × type4 + 21 × type2），counterId 范围 0x64..0x79。
//!
//! 返回：每条 record 的 (counter_id, value_type, value_u64)。

use super::constants::MEMORY_COUNTER_COUNT;

#[derive(Debug, Clone, Copy)]
pub struct RawCounter {
    pub counter_id: u32,
    pub value_type: u32, // 2 = u32, 4 = u64
    pub value: u64,
}

pub fn find_all(body: &[u8]) -> Vec<RawCounter> {
    if body.len() < 16 {
        return Vec::new();
    }
    let mut best: Vec<RawCounter> = Vec::new();
    // 在 body 中每 4 字节对齐的位置尝试启动一个 record
    let mut pos = 0;
    while pos + 16 <= body.len() {
        let start = pos;
        let mut run: Vec<RawCounter> = Vec::new();
        let mut prev_id: u32 = 0;
        loop {
            if pos + 16 > body.len() {
                break;
            }
            let count = u32::from_le_bytes([
                body[pos + 4],
                body[pos + 5],
                body[pos + 6],
                body[pos + 7],
            ]);
            if count != MEMORY_COUNTER_COUNT {
                break;
            }
            let value_type = u32::from_le_bytes([
                body[pos + 8],
                body[pos + 9],
                body[pos + 10],
                body[pos + 11],
            ]);
            let size = u32::from_le_bytes([
                body[pos + 12],
                body[pos + 13],
                body[pos + 14],
                body[pos + 15],
            ]);
            let (val_bytes, expected_size) = match value_type {
                2 => (4usize, 4u32),
                4 => (8usize, 8u32),
                _ => break,
            };
            if size != expected_size {
                break;
            }
            if pos + 16 + val_bytes > body.len() {
                break;
            }
            let id = u32::from_le_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]]);
            if !run.is_empty() && id <= prev_id {
                break;
            }
            prev_id = id;
            let value: u64 = if value_type == 2 {
                u32::from_le_bytes([
                    body[pos + 16],
                    body[pos + 17],
                    body[pos + 18],
                    body[pos + 19],
                ]) as u64
            } else {
                u64::from_le_bytes([
                    body[pos + 16],
                    body[pos + 17],
                    body[pos + 18],
                    body[pos + 19],
                    body[pos + 20],
                    body[pos + 21],
                    body[pos + 22],
                    body[pos + 23],
                ])
            };
            run.push(RawCounter {
                counter_id: id,
                value_type,
                value,
            });
            pos += 16 + val_bytes;
        }
        // 要求 run 至少有 5 条才算「有意义」的 counter 段
        if run.len() >= 5 && run.len() > best.len() {
            best = run;
        }
        pos = start + 4;
    }
    best
}

#[allow(dead_code)]
fn _suppress_unused() {}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::constants::MEMORY_COUNTER_COUNT;

    fn push_record(buf: &mut Vec<u8>, id: u32, value_type: u32, value: u64) {
        let size: u32 = if value_type == 2 { 4 } else { 8 };
        buf.extend_from_slice(&id.to_le_bytes());
        buf.extend_from_slice(&MEMORY_COUNTER_COUNT.to_le_bytes());
        buf.extend_from_slice(&value_type.to_le_bytes());
        buf.extend_from_slice(&size.to_le_bytes());
        if value_type == 2 {
            buf.extend_from_slice(&(value as u32).to_le_bytes());
        } else {
            buf.extend_from_slice(&value.to_le_bytes());
        }
    }

    #[test]
    fn finds_unity6_style_sequence() {
        // 模拟 Unity 6 实测 schema: 1×type4 + 21×type2, ids 100..121
        let mut body = vec![0u8; 32];
        body.extend_from_slice(&[0xAAu8; 8]); // 干扰数据
        let mut seq = Vec::new();
        for i in 0..22u32 {
            let id = 100 + i;
            if i == 0 {
                push_record(&mut seq, id, 4, 4_510_029);
            } else {
                push_record(&mut seq, id, 2, i as u64);
            }
        }
        body.extend_from_slice(&seq);
        let counters = find_all(&body);
        assert_eq!(counters.len(), 22);
        assert_eq!(counters[0].counter_id, 100);
        assert_eq!(counters[0].value_type, 4);
        assert_eq!(counters[0].value, 4_510_029);
        assert_eq!(counters[21].counter_id, 121);
        assert_eq!(counters[21].value_type, 2);
    }

    #[test]
    fn rejects_short_run() {
        // 只有 3 条记录 → 不算 counter 段
        let mut body = vec![0u8; 16];
        for i in 0..3u32 {
            push_record(&mut body, 100 + i, 2, 0);
        }
        let counters = find_all(&body);
        assert!(counters.len() < 5);
    }
}