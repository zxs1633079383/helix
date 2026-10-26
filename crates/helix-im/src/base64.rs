//! 标准 base64 解码（内联）—— helix-im 自带，**不** import helix-driver-host。
//!
//! ## 为什么内联（HX-C001 依赖铁律）
//!
//! 编码 `base64_encode` 住在 `helix-driver-host`（`http_envelope.rs`，ADR-007 双端共享信封）。
//! helix-im 是业务模块层，依赖方向单向 adapter → core/业务，**永不反向**——所以不能从
//! host import 配套的 decode。两条 ADR-007-legal 路：im 自带 inline decode（本文件，最小爆炸
//! 半径、零新 dep）/ 把 encode+decode 挪 core（待第三消费方出现再议）。表与 host encode 镜像：
//! `ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/`，`=` padding。
//!
//! ## 边界零信任（HX-C 不变量 4）
//!
//! 输入是 sync 信封里的 base64 串（外部 wire）→ 解码失败走 `Result::Err`，**零 unwrap/expect**，
//! 不 panic、不静默吞数据。

/// 字符 → 6-bit 值；非字母表字符返回 `None`（含 `=` padding，由调用方剥离后才进此函数）。
/// 逐字符 match 等价标准表 `ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/`
/// （与 host `base64_encode` 同表），无需具化字母表常量。
fn sextet(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// 解码标准 base64（含 `=` padding）。非法字符 / 长度不合法 → `Err(String)`（边界零信任）。
pub fn decode(s: &str) -> Result<Vec<u8>, String> {
    // 剥尾部 padding；padding 只允许出现在末尾（标准 base64 不变量）。
    let bytes = s.as_bytes();
    let body_len = bytes.iter().take_while(|&&c| c != b'=').count();
    let pad_len = bytes.len() - body_len;
    if pad_len > 2 || bytes[body_len..].iter().any(|&c| c != b'=') {
        return Err(format!("invalid base64 padding (pad_len={pad_len})"));
    }

    let core = &bytes[..body_len];
    let mut out = Vec::with_capacity(core.len() / 4 * 3 + 3);
    let mut acc: u32 = 0;
    let mut nbits: u32 = 0;
    for &c in core {
        let v = sextet(c).ok_or_else(|| format!("invalid base64 char: {c:#x}"))?;
        acc = (acc << 6) | v as u32;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
        }
    }
    // 标准 base64：尾部残余 bit 必须为 0（来自 padding 对应的空高位）。
    if nbits > 0 && (acc & ((1 << nbits) - 1)) != 0 {
        return Err("invalid base64 trailing bits".to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与 host `base64_encode` 同表的镜像编码（仅测试用，验证 round-trip）。
    fn encode(bytes: &[u8]) -> String {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            out.push(if chunk.len() > 1 {
                TABLE[((n >> 6) & 63) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                TABLE[(n & 63) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    #[test]
    fn round_trip_arbitrary_lengths() {
        for len in 0..32usize {
            let data: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_mul(31)).collect();
            let encoded = encode(&data);
            assert_eq!(decode(&encoded).unwrap(), data, "round-trip len={len}");
        }
    }

    #[test]
    fn decode_known_vector() {
        assert_eq!(decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode("Zm9vYmFy").unwrap(), b"foobar");
        assert_eq!(decode("").unwrap(), b"");
    }

    #[test]
    fn rejects_invalid_chars() {
        assert!(decode("aGVsbG8*").is_err());
        assert!(decode("not base64 ###").is_err());
    }
}
