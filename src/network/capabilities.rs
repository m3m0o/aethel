use std::fs;

use anyhow::{Context, Result};

use super::snapshot::CapabilityState;

pub fn inspect() -> Result<CapabilityState> {
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status")
            .context("failed to read process capabilities from /proc/self/status")?;
        let effective = status
            .lines()
            .find_map(|line| line.strip_prefix("CapEff:\t"))
            .ok_or_else(|| anyhow::anyhow!("/proc/self/status does not contain CapEff"))?;
        let bits = u128::from_str_radix(effective, 16)
            .with_context(|| format!("invalid CapEff value: {effective}"))?;
        Ok(CapabilityState {
            net_admin: bits & (1 << 12) != 0,
            net_raw: bits & (1 << 13) != 0,
        })
    }

    #[cfg(not(target_os = "linux"))]
    {
        Err(anyhow::anyhow!(
            "capability inspection is supported only on Linux"
        ))
    }
}
