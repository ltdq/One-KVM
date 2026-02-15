//! ATX Controller
//!
//! High-level controller for ATX power management with flexible hardware binding.
//! Each action (power short, power long, reset) can be configured independently.

use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::executor::{timing, AtxKeyExecutor};
use super::led::LedSensor;
use super::miot::MiotBackend;
use super::types::{AtxDriverType, AtxKeyConfig, AtxStatusConfig, AtxStatusDriverType, MiotConfig, AtxState, PowerStatus};
use crate::error::{AppError, Result};

/// ATX power control configuration
#[derive(Debug, Clone)]
pub struct AtxControllerConfig {
    /// Whether ATX is enabled
    pub enabled: bool,
    /// Power button configuration (used for both short and long press)
    pub power: AtxKeyConfig,
    /// Reset button configuration
    pub reset: AtxKeyConfig,
    /// Status detection configuration
    pub status: AtxStatusConfig,
    /// MiIoT connection settings (shared by all keys using driver=Miot)
    pub miot: MiotConfig,
}

impl Default for AtxControllerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            power: AtxKeyConfig::default(),
            reset: AtxKeyConfig::default(),
            status: AtxStatusConfig::default(),
            miot: MiotConfig::default(),
        }
    }
}

/// Check if any component uses the MiIoT backend
fn needs_miot_backend(config: &AtxControllerConfig) -> bool {
    config.power.driver == AtxDriverType::Miot
        || config.reset.driver == AtxDriverType::Miot
        || config.status.driver == AtxStatusDriverType::Miot
}

/// Internal state holding all ATX components
/// Grouped together to reduce lock acquisitions
struct AtxInner {
    config: AtxControllerConfig,
    power_executor: Option<AtxKeyExecutor>,
    reset_executor: Option<AtxKeyExecutor>,
    led_sensor: Option<LedSensor>,
    /// MiIoT backend (shared by components using driver=Miot)
    miot_backend: Option<MiotBackend>,
}

/// ATX Controller
///
/// Manages ATX power control through independent executors for each action.
/// Supports hot-reload of configuration.
pub struct AtxController {
    /// Single lock for all internal state to reduce lock contention
    inner: RwLock<AtxInner>,
}

impl AtxController {
    /// Create a new ATX controller with the specified configuration
    pub fn new(config: AtxControllerConfig) -> Self {
        Self {
            inner: RwLock::new(AtxInner {
                config,
                power_executor: None,
                reset_executor: None,
                led_sensor: None,
                miot_backend: None,
            }),
        }
    }

    /// Create a disabled ATX controller
    pub fn disabled() -> Self {
        Self::new(AtxControllerConfig::default())
    }

    /// Initialize the ATX controller and its executors
    pub async fn init(&self) -> Result<()> {
        let mut inner = self.inner.write().await;

        if !inner.config.enabled {
            info!("ATX disabled in configuration");
            return Ok(());
        }

        info!("Initializing ATX controller");

        // Initialize MiIoT backend if any component uses it
        if needs_miot_backend(&inner.config) {
            if inner.config.miot.is_configured() {
                info!("ATX using MiIoT backend for device {}", inner.config.miot.did);
                let backend = MiotBackend::new(inner.config.miot.clone());
                if let Err(e) = backend.init().await {
                    warn!("Failed to initialize MiIoT backend: {}", e);
                } else {
                    info!("MiIoT backend initialized successfully");
                    inner.miot_backend = Some(backend);
                }
            } else {
                warn!("Component(s) configured with MiIoT driver but MiIoT connection (DID) not set");
            }
        }

        // Initialize power executor (GPIO/USB relay only)
        if inner.config.power.driver != AtxDriverType::Miot && inner.config.power.is_configured() {
            let mut executor = AtxKeyExecutor::new(inner.config.power.clone());
            if let Err(e) = executor.init().await {
                warn!("Failed to initialize power executor: {}", e);
            } else {
                info!(
                    "Power executor initialized: {:?} on {} pin {}",
                    inner.config.power.driver, inner.config.power.device, inner.config.power.pin
                );
                inner.power_executor = Some(executor);
            }
        }

        // Initialize reset executor (GPIO/USB relay only)
        if inner.config.reset.driver != AtxDriverType::Miot && inner.config.reset.is_configured() {
            let mut executor = AtxKeyExecutor::new(inner.config.reset.clone());
            if let Err(e) = executor.init().await {
                warn!("Failed to initialize reset executor: {}", e);
            } else {
                info!(
                    "Reset executor initialized: {:?} on {} pin {}",
                    inner.config.reset.driver, inner.config.reset.device, inner.config.reset.pin
                );
                inner.reset_executor = Some(executor);
            }
        }

        // Initialize LED sensor (only if status driver is Led)
        if inner.config.status.driver == AtxStatusDriverType::Led && inner.config.status.is_configured() {
            let led_config = super::types::AtxLedConfig {
                enabled: true,
                gpio_chip: inner.config.status.gpio_chip.clone(),
                gpio_pin: inner.config.status.gpio_pin,
                inverted: inner.config.status.inverted,
            };
            let mut sensor = LedSensor::new(led_config);
            if let Err(e) = sensor.init().await {
                warn!("Failed to initialize LED sensor: {}", e);
            } else {
                info!(
                    "LED sensor initialized on {} pin {}",
                    inner.config.status.gpio_chip, inner.config.status.gpio_pin
                );
                inner.led_sensor = Some(sensor);
            }
        }

        info!("ATX controller initialized successfully");
        Ok(())
    }

    /// Reload the ATX controller with new configuration
    ///
    /// This is called when configuration changes and supports hot-reload.
    pub async fn reload(&self, new_config: AtxControllerConfig) -> Result<()> {
        info!("Reloading ATX controller with new configuration");

        // Shutdown existing executors
        self.shutdown_internal().await?;

        // Update configuration and re-initialize
        {
            let mut inner = self.inner.write().await;
            inner.config = new_config;
        }

        // Re-initialize
        self.init().await?;

        info!("ATX controller reloaded successfully");
        Ok(())
    }

    /// Get current ATX state (single lock acquisition)
    pub async fn state(&self) -> AtxState {
        let inner = self.inner.read().await;

        let power_status = self.get_power_status_inner(&inner).await;

        let power_configured = inner.config.power.is_configured()
            && (inner.config.power.driver != AtxDriverType::Miot
                || inner.miot_backend.is_some());
        let reset_configured = inner.config.reset.is_configured()
            && (inner.config.reset.driver != AtxDriverType::Miot
                || inner.miot_backend.is_some());

        let status_supported = match inner.config.status.driver {
            AtxStatusDriverType::Led => inner.led_sensor.as_ref().map(|s| s.is_initialized()).unwrap_or(false),
            AtxStatusDriverType::Miot => inner.miot_backend.is_some(),
            AtxStatusDriverType::None => false,
        };

        AtxState {
            available: inner.config.enabled,
            power_configured,
            reset_configured,
            power_status,
            status_supported,
        }
    }

    /// Get current state as SystemEvent
    pub async fn current_state_event(&self) -> crate::events::SystemEvent {
        let state = self.state().await;
        crate::events::SystemEvent::AtxStateChanged {
            power_status: state.power_status,
        }
    }

    /// Check if ATX is available
    pub async fn is_available(&self) -> bool {
        let inner = self.inner.read().await;
        inner.config.enabled
    }

    /// Check if power button is configured and initialized
    pub async fn is_power_ready(&self) -> bool {
        let inner = self.inner.read().await;
        inner
            .power_executor
            .as_ref()
            .map(|e| e.is_initialized())
            .unwrap_or(false)
    }

    /// Check if reset button is configured and initialized
    pub async fn is_reset_ready(&self) -> bool {
        let inner = self.inner.read().await;
        inner
            .reset_executor
            .as_ref()
            .map(|e| e.is_initialized())
            .unwrap_or(false)
    }

    /// Short press power button (turn on or graceful shutdown)
    pub async fn power_short(&self) -> Result<()> {
        let inner = self.inner.read().await;

        // MiIoT driver: determine value based on current power status
        if inner.config.power.driver == AtxDriverType::Miot {
            let miot = inner.miot_backend.as_ref()
                .ok_or_else(|| AppError::Internal("MiIoT backend not initialized".to_string()))?;

            let current_status = self.get_power_status_inner(&inner).await;
            let (prop, value) = match current_status {
                PowerStatus::On => (&inner.config.power.off_prop, &inner.config.power.off_value),
                PowerStatus::Off | PowerStatus::Unknown => (&inner.config.power.prop, &inner.config.power.value),
            };
            info!("ATX MiIoT: Short press power (status={:?}, set {}={})", current_status, prop, value);
            return miot.set_prop(prop, value).await;
        }

        // GPIO/USB relay: pulse power pin
        let executor = inner
            .power_executor
            .as_ref()
            .ok_or_else(|| AppError::Internal("Power button not configured".to_string()))?;

        info!(
            "ATX: Short press power button ({}ms)",
            timing::SHORT_PRESS.as_millis()
        );
        executor.pulse(timing::SHORT_PRESS).await
    }

    /// Long press power button (sends off_prop=off_value for MiIoT)
    pub async fn power_long(&self) -> Result<()> {
        let inner = self.inner.read().await;

        // MiIoT driver: send configured off_prop=off_value
        if inner.config.power.driver == AtxDriverType::Miot {
            let miot = inner.miot_backend.as_ref()
                .ok_or_else(|| AppError::Internal("MiIoT backend not initialized".to_string()))?;
            let prop = &inner.config.power.off_prop;
            let value = &inner.config.power.off_value;
            info!("ATX MiIoT: Force power off (set {}={})", prop, value);
            return miot.set_prop(prop, value).await;
        }

        // GPIO/USB relay: long pulse power pin
        let executor = inner
            .power_executor
            .as_ref()
            .ok_or_else(|| AppError::Internal("Power button not configured".to_string()))?;

        info!(
            "ATX: Long press power button ({}ms)",
            timing::LONG_PRESS.as_millis()
        );
        executor.pulse(timing::LONG_PRESS).await
    }

    /// Press reset button
    pub async fn reset(&self) -> Result<()> {
        let inner = self.inner.read().await;

        // MiIoT driver: send configured prop=value
        if inner.config.reset.driver == AtxDriverType::Miot {
            let miot = inner.miot_backend.as_ref()
                .ok_or_else(|| AppError::Internal("MiIoT backend not initialized".to_string()))?;
            let prop = &inner.config.reset.prop;
            let value = &inner.config.reset.value;
            info!("ATX MiIoT: Reset (set {}={})", prop, value);
            return miot.set_prop(prop, value).await;
        }

        // GPIO/USB relay: pulse reset pin
        let executor = inner
            .reset_executor
            .as_ref()
            .ok_or_else(|| AppError::Internal("Reset button not configured".to_string()))?;

        info!(
            "ATX: Press reset button ({}ms)",
            timing::RESET_PRESS.as_millis()
        );
        executor.pulse(timing::RESET_PRESS).await
    }

    /// Get current power status from status detection (internal helper, caller holds read lock)
    async fn get_power_status_inner(&self, inner: &AtxInner) -> PowerStatus {
        match inner.config.status.driver {
            AtxStatusDriverType::Miot => {
                if let Some(miot) = inner.miot_backend.as_ref() {
                    miot.get_power_status(&inner.config.status.prop, &inner.config.status.on_value)
                        .await
                        .unwrap_or(PowerStatus::Unknown)
                } else {
                    PowerStatus::Unknown
                }
            }
            AtxStatusDriverType::Led => {
                match inner.led_sensor.as_ref() {
                    Some(sensor) => sensor.read().await.unwrap_or(PowerStatus::Unknown),
                    None => PowerStatus::Unknown,
                }
            }
            AtxStatusDriverType::None => PowerStatus::Unknown,
        }
    }

    /// Get current power status from status detection
    pub async fn power_status(&self) -> Result<PowerStatus> {
        let inner = self.inner.read().await;
        Ok(self.get_power_status_inner(&inner).await)
    }

    /// Shutdown the ATX controller
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down ATX controller");
        self.shutdown_internal().await?;
        info!("ATX controller shutdown complete");
        Ok(())
    }

    /// Internal shutdown helper
    async fn shutdown_internal(&self) -> Result<()> {
        let mut inner = self.inner.write().await;

        // Shutdown MiIoT backend
        if let Some(mut backend) = inner.miot_backend.take() {
            backend.shutdown().await.ok();
        }

        // Shutdown power executor
        if let Some(mut executor) = inner.power_executor.take() {
            executor.shutdown().await.ok();
        }

        // Shutdown reset executor
        if let Some(mut executor) = inner.reset_executor.take() {
            executor.shutdown().await.ok();
        }

        // Shutdown LED sensor
        if let Some(mut sensor) = inner.led_sensor.take() {
            sensor.shutdown().await.ok();
        }

        Ok(())
    }
}

impl Drop for AtxController {
    fn drop(&mut self) {
        debug!("ATX controller dropped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_controller_config_default() {
        let config = AtxControllerConfig::default();
        assert!(!config.enabled);
        assert!(!config.power.is_configured());
        assert!(!config.reset.is_configured());
        assert!(!config.status.is_configured());
    }

    #[test]
    fn test_controller_creation() {
        let controller = AtxController::disabled();
        assert!(controller.inner.try_read().is_ok());
    }

    #[tokio::test]
    async fn test_controller_disabled_state() {
        let controller = AtxController::disabled();
        let state = controller.state().await;
        assert!(!state.available);
        assert!(!state.power_configured);
        assert!(!state.reset_configured);
    }

    #[tokio::test]
    async fn test_controller_init_disabled() {
        let controller = AtxController::disabled();
        let result = controller.init().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_controller_is_available() {
        let controller = AtxController::disabled();
        assert!(!controller.is_available().await);

        let config = AtxControllerConfig {
            enabled: true,
            ..Default::default()
        };
        let controller = AtxController::new(config);
        assert!(controller.is_available().await);
    }
}
