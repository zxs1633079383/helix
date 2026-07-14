#[cfg(unix)]
pub(super) fn process_resident_memory_bytes() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage 在成功返回前完整初始化调用方提供的 rusage 指针；指针来自有效栈内存。
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    // SAFETY: 仅在 getrusage 返回 0 后读取，POSIX 契约保证结构体已初始化。
    let max_rss = unsafe { usage.assume_init() }.ru_maxrss.max(0) as u64;
    #[cfg(target_vendor = "apple")]
    return Some(max_rss);
    #[cfg(not(target_vendor = "apple"))]
    return max_rss.checked_mul(1024);
}

#[cfg(not(unix))]
pub(super) fn process_resident_memory_bytes() -> Option<u64> {
    None
}
