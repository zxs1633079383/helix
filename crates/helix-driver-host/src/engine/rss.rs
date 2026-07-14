#[cfg(target_vendor = "apple")]
pub(super) fn process_resident_memory_bytes() -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_taskinfo>() as i32;
    // SAFETY: proc_pidinfo 在返回完整结构体大小时初始化 info；pid 是当前进程。
    let written = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    // SAFETY: 仅在 proc_pidinfo 返回完整结构体大小后读取。
    Some(unsafe { info.assume_init() }.pti_resident_size)
}

#[cfg(target_os = "linux")]
pub(super) fn process_resident_memory_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident_pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    // SAFETY: sysconf 是无副作用查询；负值表示查询失败。
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    (page_size > 0)
        .then_some(resident_pages)
        .and_then(|pages| pages.checked_mul(page_size as u64))
}

#[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
pub(super) fn process_resident_memory_bytes() -> Option<u64> {
    None
}
