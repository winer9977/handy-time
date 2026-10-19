//! 短 ID 生成
//!
//! 桌面挂件数据量小但条目多，用 12 位 base32 短码替代 36 位 UUID：
//! JSON 更短、UI 里显示友好。
//!
//! ★ 为什么从 8 位（40 bit）加宽到 12 位（56 bit）：这些 id **永不复用** ——
//! 归档分片、回收站里的行都还占着自己的 id，重复待办每完成一次就新生成一条，
//! 一个用了几年的库累计生成量会爬到十万级。而生日悖论是按**累计生成数**算的：
//! 40 bit 空间下 10 万次生成的期望碰撞 ≈ 1e10 / 2^41 ≈ 0.0045，也就是
//! "千分之一次的概率出现两条同 id" —— 同 id 会让 `find(|t| t.id == id)`
//! 与回收站的 `retain(|x| x.todo.id != tid)` 打到错误的行（改一条动两条）。
//! 实测：`no_collision_in_100k` 就是这么红的（不是测试写错，是生成器撑不住
//! 它宣称的保证）。56 bit 下同样的 10 万次期望碰撞 ≈ 7e-8，硬断言才站得住。
//!
//! 空间：7 字节(56 bit) → 12 × 5 bit（最后一位只带 1 bit 真随机，够用）。

use uuid::Uuid;

const ALPHABET: [u8; 32] = *b"abcdefghijklmnopqrstuvwxyz012345";

pub fn new() -> String {
    let bytes = Uuid::new_v4().as_bytes().to_vec();
    // 取前 7 字节（56 bit）→ 12 个 base32 字符
    let mut out = String::with_capacity(12);
    let mut bits: u64 = 0;
    let mut nbits = 0;
    for &b in &bytes[..7] {
        bits = (bits << 8) | b as u64;
        nbits += 8;
        while nbits >= 5 {
            nbits -= 5;
            out.push(ALPHABET[((bits >> nbits) & 0x1F) as usize] as char);
        }
    }
    if nbits > 0 {
        out.push(ALPHABET[((bits << (5 - nbits)) & 0x1F) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn length_and_alphabet() {
        let id = new();
        assert_eq!(id.len(), 12);
        assert!(id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }

    #[test]
    fn no_collision_in_100k() {
        // 这条以前是"掷骰子"：40 bit 空间下 10 万次约有 0.45% 概率红。
        // 加宽到 56 bit 后期望碰撞 7e-8，它才是一条**可靠**的保证，
        // 同时也是对"别再把字节数改回去"的护栏。
        let mut set = HashSet::new();
        for _ in 0..100_000 {
            assert!(set.insert(new()));
        }
    }

    #[test]
    fn ids_are_well_distributed() {
        // 采样看分布：如果哪天有人把 `bytes[..7]` 改短（尾部变成恒定填充），
        // 前缀会扎堆，这条就红。
        let prefixes: HashSet<String> = (0..2000).map(|_| new()[..4].to_string()).collect();
        // 4 位 = 20 bit → 2000 次抽样在 1e6 空间里，正常应得到 ~1998 个不同前缀
        assert!(prefixes.len() > 1900, "前 4 位分布过于集中：{}", prefixes.len());
    }
}
