use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::process::{Child, Command};

use crate::config::NetworkConfig;

pub struct NdppdProcess {
    child: Child,
    config_path: PathBuf,
}

impl NdppdProcess {
    pub async fn start(config: &NetworkConfig, executable: &Path) -> Result<Self> {
        validate_executable(executable)?;
        fs::create_dir_all(&config.state.root).with_context(|| {
            format!(
                "failed to create ndppd state directory {}",
                config.state.root.display()
            )
        })?;
        let config_path = config.state.root.join("ndppd.conf");
        write_config(&config_path, config)?;

        let child = Command::new(executable)
            .arg("-c")
            .arg(&config_path)
            .kill_on_drop(true)
            .spawn()
            .with_context(|| {
                format!("failed to start ndppd executable {}", executable.display())
            })?;
        Ok(Self { child, config_path })
    }

    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    pub async fn wait(mut self) -> Result<()> {
        let status = self
            .child
            .wait()
            .await
            .context("failed to wait for ndppd")?;
        self.remove_config()?;
        if status.success() {
            Ok(())
        } else {
            anyhow::bail!("ndppd exited with status {status}")
        }
    }

    pub async fn stop(mut self) -> Result<()> {
        self.child
            .start_kill()
            .context("failed to signal ndppd for shutdown")?;
        let result = self
            .child
            .wait()
            .await
            .context("failed to wait for ndppd shutdown");
        self.remove_config()?;
        result.map(|_| ())
    }

    fn remove_config(&self) -> Result<()> {
        match fs::remove_file(&self.config_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to remove temporary ndppd config {}",
                    self.config_path.display()
                )
            }),
        }
    }
}

fn validate_executable(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect ndppd executable {}", path.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("ndppd executable path is not a file: {}", path.display());
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o111 == 0 {
        anyhow::bail!("ndppd executable is not executable: {}", path.display());
    }
    Ok(())
}

fn write_config(path: &Path, config: &NetworkConfig) -> Result<()> {
    let contents = format!(
        "route-ttl 30000\n\nproxy {} {{\n    router yes\n    timeout 500\n    ttl 30000\n    rule {} {{\n        static\n    }}\n}}\n",
        config.network.interface, config.network.prefix
    );
    fs::write(path, contents)
        .with_context(|| format!("failed to write temporary ndppd config {}", path.display()))
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(test)]
mod tests {
    use super::write_config;
    use crate::config::{
        NdpBackend, NetworkConfig, NetworkSection, RouteSection, StateSection, SysctlSection,
    };
    use std::path::Path;

    #[test]
    fn generated_config_contains_only_configured_network_values() {
        let config = NetworkConfig {
            version: 1,
            network: NetworkSection {
                interface: "net0".to_owned(),
                prefix: "2001:db8:1::/64".to_owned(),
                loopback: "lo".to_owned(),
                backend: NdpBackend::Ndppd,
            },
            route: RouteSection::default(),
            sysctl: SysctlSection::default(),
            state: StateSection {
                root: "/tmp/aethel-test-state".into(),
            },
        };
        let path = Path::new("/tmp/aethel-ndppd-test.conf");
        write_config(path, &config).unwrap();
        let contents = std::fs::read_to_string(path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(contents.contains("proxy net0"));
        assert!(contents.contains("rule 2001:db8:1::/64"));
    }
}
