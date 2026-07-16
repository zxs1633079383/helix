//! IM 状态机使用的值对象；只承载确定性数据，不含 I/O。

/// mattermost 26 字符 `[a-z0-9]` ID；Copy newtype，热路径零分配 key（HX-C005）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id26([u8; 26]);

impl Id26 {
    /// 边界零信任构造：仅恰 26 字节且全 `[a-z0-9]` 时 Some。
    pub fn from_str(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 26
            || !b
                .iter()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            return None;
        }
        let mut arr = [0u8; 26];
        arr.copy_from_slice(b);
        Some(Self(arr))
    }

    /// 零拷贝取字符串视图。
    pub fn as_str(&self) -> &str {
        // SAFETY: Id26 只能经 from_str 构造，内部恒为合法 UTF-8。
        unsafe { std::str::from_utf8_unchecked(&self.0) }
    }

    pub fn as_bytes(&self) -> &[u8; 26] {
        &self.0
    }

    /// 派生确定性 u64（FNV-1a，非加密）。
    pub fn hash_u64(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in &self.0 {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }
}

/// 频道 ID（mattermost 26 字符 ID，不透明 Copy）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChannelId(pub Id26);

impl ChannelId {
    pub fn from_str(s: &str) -> Option<Self> {
        Id26::from_str(s).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn as_bytes(&self) -> &[u8; 26] {
        self.0.as_bytes()
    }

    pub fn hash_u64(&self) -> u64 {
        self.0.hash_u64()
    }
}

/// Server 消息 ID（echo 到时的真实 ID）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServerId(pub Id26);

impl ServerId {
    pub fn from_str(s: &str) -> Option<Self> {
        Id26::from_str(s).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Helix 铸造并持有的临时消息 ID（乐观落库 PK / HTTP / echo 对账锚）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TemporaryId(pub String);

impl TemporaryId {
    /// 用 step 时钟与 module 内单调序号铸造确定性 ID。
    ///
    /// 相同 tick/clock 序列产生相同结果；单个 ImModule 生命周期内 O(1)
    /// 分配且不重复。公开 command payload 不参与 ID 选择。
    pub(crate) fn mint(now_ms: u64, sequence: u64) -> Self {
        Self(format!("helix_tmp_{now_ms:016x}_{sequence:016x}"))
    }
}

/// per-channel 事件序号（单调递增）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Seq(pub u64);

impl Seq {
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendStatus {
    Local,
    Sending,
    Sent,
    UnSend,
}

/// 同一 channel 同时只允许一个在途 sync Http。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InflightSync(pub helix_core::Correlation);

/// per-channel 游标（单调推进，MAX guard）。
#[derive(Debug, Clone, Copy)]
pub struct Cursor {
    pub value: Seq,
}

impl Cursor {
    pub fn new(value: Seq) -> Self {
        Self { value }
    }

    pub fn try_advance(&mut self, next: Seq) -> bool {
        if next > self.value {
            self.value = next;
            true
        } else {
            false
        }
    }

    pub fn value(&self) -> Seq {
        self.value
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Disconnected,
    Connecting,
    Connected,
    Closing,
}
