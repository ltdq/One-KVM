//! MiIoT Smart Plug Backend for ATX Control
//!
//! Uses mijiaAPI CLI tool to control MiIoT smart plug devices (e.g., LKRQ开机卡)
//! as an alternative ATX power control method.
//!
//! The backend is generic: each ATX key (power, reset) specifies its own
//! `prop` (property name) and `value` via `AtxKeyConfig`. The backend simply
//! executes `mijiaAPI get/set` with the given parameters.
//!
//! # Example Property Mapping (LKRQ card)
//!
//! - Power button short press: prop=`on`, value=`True`; long press: off_prop=`on`, off_value=`False`
//! - Reset button: prop=`name`, value=`1` (reset command)

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::process::Command;
use std::process::Stdio;
use tracing::{debug, info, warn};

use super::types::{MiotConfig, PowerStatus};
use crate::error::{AppError, Result};

/// Timeout for each mijiaAPI subprocess call
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// MiIoT backend for ATX power control via smart plug
///
/// Communicates with MiIoT devices through the `mijiaAPI` CLI tool.
/// All operations are async subprocess calls.
pub struct MiotBackend {
    config: MiotConfig,
    initialized: AtomicBool,
}

impl MiotBackend {
    /// Create a new MiIoT backend with the given configuration
    pub fn new(config: MiotConfig) -> Self {
        Self {
            config,
            initialized: AtomicBool::new(false),
        }
    }

    /// Check if the backend is configured
    #[allow(dead_code)]
    pub fn is_configured(&self) -> bool {
        self.config.is_configured()
    }

    /// Check if the backend is initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Relaxed)
    }

    /// Initialize the MiIoT backend
    ///
    /// Marks the backend as ready without blocking on device verification.
    pub async fn init(&self) -> Result<()> {
        if !self.config.is_configured() {
            debug!("MiIoT backend not configured, skipping init");
            return Ok(());
        }

        info!(
            "Initializing MiIoT backend for device {} using command '{}'",
            self.config.did, self.config.command
        );

        self.initialized.store(true, Ordering::Relaxed);
        info!("MiIoT backend initialized (device connectivity will be verified on first use)");
        Ok(())
    }

    /// Execute a set command: `mijiaAPI set --did <DID> --prop_name <prop> --value <value>`
    pub async fn set_prop(&self, prop: &str, value: &str) -> Result<()> {
        info!("MiIoT: set {}={} on device {}", prop, value, self.config.did);
        self.run_set(prop, value).await?;
        debug!("MiIoT: set command sent successfully");
        Ok(())
    }

    /// Get a property value: `mijiaAPI get --did <DID> --prop_name <prop>`
    ///
    /// Returns the raw output string.
    #[allow(dead_code)]
    pub async fn get_prop(&self, prop: &str) -> Result<String> {
        self.run_get(prop).await
    }

    /// Get power status by reading a boolean property.
    ///
    /// Reads `prop` and compares the parsed value against `on_value`.
    /// If the parsed value matches `on_value` → On, otherwise → Off.
    pub async fn get_power_status(&self, prop: &str, on_value: &str) -> Result<PowerStatus> {
        if !self.is_initialized() {
            return Ok(PowerStatus::Unknown);
        }

        let output = self.run_get(prop).await?;
        let status = parse_power_status(&output, on_value);
        debug!("MiIoT device {} prop={} status: {:?}", self.config.did, prop, status);
        Ok(status)
    }

    /// Shutdown the MiIoT backend
    pub async fn shutdown(&mut self) -> Result<()> {
        self.initialized.store(false, Ordering::Relaxed);
        debug!("MiIoT backend shutdown complete");
        Ok(())
    }

    // ========== Internal helpers ==========

    /// Run `mijiaAPI get` command and return the raw output
    async fn run_get(&self, prop_name: &str) -> Result<String> {
        debug!("mijiaAPI get: did={} prop={}", self.config.did, prop_name);
        let child = Command::new(&self.config.command)
            .args(["get", "--did", &self.config.did, "--prop_name", prop_name])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                AppError::Internal(format!(
                    "Failed to spawn '{}': {}. Is mijiaAPI installed and in PATH?",
                    self.config.command, e
                ))
            })?;

        let output = tokio::time::timeout(COMMAND_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| {
                AppError::Internal(format!(
                    "mijiaAPI get timed out after {}s",
                    COMMAND_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|e| {
                AppError::Internal(format!("mijiaAPI get wait failed: {}", e))
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        debug!("mijiaAPI get result: exit={} stdout='{}' stderr='{}'",
            output.status.code().unwrap_or(-1), stdout.trim(), stderr.trim());

        if !output.status.success() {
            return Err(AppError::Internal(format!(
                "mijiaAPI get failed (exit {}): stdout={}, stderr={}",
                output.status.code().unwrap_or(-1),
                stdout.trim(),
                stderr.trim()
            )));
        }

        Ok(stdout)
    }

    /// Run `mijiaAPI set` command
    async fn run_set(&self, prop_name: &str, value: &str) -> Result<()> {
        debug!("mijiaAPI set: did={} prop={} value={}", self.config.did, prop_name, value);
        let child = Command::new(&self.config.command)
            .args([
                "set",
                "--did",
                &self.config.did,
                "--prop_name",
                prop_name,
                "--value",
                value,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                AppError::Internal(format!(
                    "Failed to spawn '{}': {}. Is mijiaAPI installed and in PATH?",
                    self.config.command, e
                ))
            })?;

        let output = tokio::time::timeout(COMMAND_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| {
                warn!("mijiaAPI set timed out after {}s, killing process", COMMAND_TIMEOUT.as_secs());
                AppError::Internal(format!(
                    "mijiaAPI set timed out after {}s",
                    COMMAND_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|e| {
                AppError::Internal(format!("mijiaAPI set wait failed: {}", e))
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        debug!("mijiaAPI set result: exit={} stdout='{}' stderr='{}'",
            output.status.code().unwrap_or(-1), stdout.trim(), stderr.trim());

        if !output.status.success() {
            return Err(AppError::Internal(format!(
                "mijiaAPI set {}={} failed (exit {}): stdout={}, stderr={}",
                prop_name,
                value,
                output.status.code().unwrap_or(-1),
                stdout.trim(),
                stderr.trim()
            )));
        }

        Ok(())
    }
}

impl Drop for MiotBackend {
    fn drop(&mut self) {
        debug!("MiIoT backend dropped");
    }
}

/// Parse power status from mijiaAPI output.
///
/// Expected output format: `设备名 (DID) 的 prop_name 值为 <value>`
/// Extracts the value after "值为" and compares with `on_value`.
fn parse_power_status(output: &str, on_value: &str) -> PowerStatus {
    if let Some(parsed) = parse_value_from_output(output) {
        if parsed.eq_ignore_ascii_case(on_value) {
            PowerStatus::On
        } else {
            PowerStatus::Off
        }
    } else {
        warn!("Could not parse MiIoT output: '{}'", output.trim());
        PowerStatus::Unknown
    }
}

/// Extract the value part from mijiaAPI output (after "值为").
///
/// Scans lines in reverse to skip warning lines.
fn parse_value_from_output(output: &str) -> Option<String> {
    for line in output.lines().rev() {
        let line = line.trim();
        if let Some(pos) = line.rfind("值为") {
            let value_part = line[pos + "值为".len()..].trim();
            if !value_part.is_empty() {
                return Some(value_part.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_power_status_on() {
        let output = "LKRQ电脑开机卡 (2094828328) 的 on 值为 True\n";
        assert_eq!(parse_power_status(output, "True"), PowerStatus::On);
    }

    #[test]
    fn test_parse_power_status_off() {
        let output = "LKRQ电脑开机卡 (2094828328) 的 on 值为 False\n";
        assert_eq!(parse_power_status(output, "True"), PowerStatus::Off);
    }

    #[test]
    fn test_parse_power_status_with_warning() {
        let output = "2026-02-15 00:27:46.603 - mijiaAPI - WARNING: 同时提供了 did 和 dev_name 参数，将忽略 dev_name\nLKRQ电脑开机卡 (2094828328) 的 on 值为 False\n";
        assert_eq!(parse_power_status(output, "True"), PowerStatus::Off);
    }

    #[test]
    fn test_parse_power_status_unknown() {
        assert_eq!(parse_power_status("some unexpected output", "True"), PowerStatus::Unknown);
        assert_eq!(parse_power_status("", "True"), PowerStatus::Unknown);
    }

    #[test]
    fn test_parse_value_from_output() {
        let output = "LKRQ电脑开机卡 (2094828328) 的 on 值为 True\n";
        assert_eq!(parse_value_from_output(output), Some("True".to_string()));

        assert_eq!(parse_value_from_output("no match"), None);
        assert_eq!(parse_value_from_output(""), None);
    }

    #[test]
    fn test_miot_backend_creation() {
        let config = MiotConfig::default();
        let backend = MiotBackend::new(config);
        assert!(!backend.is_configured());
        assert!(!backend.is_initialized());
    }

    #[test]
    fn test_miot_backend_configured() {
        let config = MiotConfig {
            did: "2094828328".to_string(),
            command: "mijiaAPI".to_string(),
            auth_path: String::new(),
        };
        let backend = MiotBackend::new(config);
        assert!(backend.is_configured());
        assert!(!backend.is_initialized());
    }
}
