//! Unity Profiler marker category 映射
//!
//! `groupFlags >> 16` 给出 category ID；映射来自 librashuai。

use super::constants::category_name as cn;

/// Marker category ID（来自 groupFlags 高 16 位）。
pub type CategoryId = u16;

/// 把 category ID 解析成可读分类名；未知 ID 返回 `None`。
pub fn name(id: CategoryId) -> Option<&'static str> {
    let n = cn(id);
    if n.is_empty() { None } else { Some(n) }
}

/// 已知的 category 集合，便于 `accumulate_cpu` 累计时跳过未知项。
pub fn is_known(id: CategoryId) -> bool {
    matches!(
        id,
        0 | 1 | 4 | 5 | 6 | 17 | 18 | 26 | 27 | 33 | 35
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_categories_resolve() {
        assert_eq!(name(0), Some("Render"));
        assert_eq!(name(1), Some("Scripts"));
        assert_eq!(name(17), Some("GC"));
        assert_eq!(name(35), Some("UI Details"));
    }

    #[test]
    fn unknown_returns_none() {
        assert_eq!(name(999), None);
        assert!(!is_known(999));
    }
}