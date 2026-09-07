use std::fs;

#[cfg(target_os = "linux")]
pub fn host_summary() -> Result<String, String> {
    let mut interfaces = fs::read_dir("/sys/class/net")
        .map_err(|error| format!("failed to inspect network interfaces: {error}"))?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .map_err(|error| format!("failed to inspect network interface: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    interfaces.sort();

    let forwarding = read_sysctl("/proc/sys/net/ipv6/conf/all/forwarding")?;
    let ip_nonlocal_bind = read_sysctl("/proc/sys/net/ipv6/ip_nonlocal_bind")?;

    Ok(format!(
        "network state: interfaces=[{}], ipv6.forwarding={}, ipv6.ip_nonlocal_bind={}",
        interfaces.join(", "),
        forwarding,
        ip_nonlocal_bind
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn host_summary() -> Result<String, String> {
    Err("network check without --config is supported only on Linux".to_owned())
}

#[cfg(target_os = "linux")]
fn read_sysctl(path: &str) -> Result<String, String> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .map_err(|error| format!("failed to read {path}: {error}"))
}
