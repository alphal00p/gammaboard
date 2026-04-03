pub(crate) fn current_rss_bytes() -> Option<i64> {
    #[cfg(target_os = "linux")]
    {
        let contents = std::fs::read_to_string("/proc/self/status").ok()?;
        for line in contents.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                let value = rest.split_whitespace().next()?;
                let kib = value.parse::<i64>().ok()?;
                return kib.checked_mul(1024);
            }
        }
        None
    }

    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}
