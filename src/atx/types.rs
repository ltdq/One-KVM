//! ATX data types and structures
//!
//! Defines the configuration and state types for the flexible ATX power control system.
//! Each ATX action (power, reset) can be independently configured with different hardware.

use serde::{Deserialize, Serialize};
use typeshare::typeshare;

/// Power status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PowerStatus {
    /// Power is on
    On,
    /// Power is off
    Off,
    /// Power status unknown (no status detection configured)
    Unknown,
}

impl Default for PowerStatus {
    fn default() -> Self {
        Self::Unknown
    }
}

/// Driver type for ATX key operations
#[typeshare]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AtxDriverType {
    /// GPIO control via Linux character device
    Gpio,
    /// USB HID relay module
    UsbRelay,
    /// MiIoT smart plug (开机卡)
    Miot,
    /// Disabled / Not configured
    None,
}

impl Default for AtxDriverType {
    fn default() -> Self {
        Self::None
    }
}

/// Active level for GPIO pins
#[typeshare]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActiveLevel {
    /// Active high (default for most cases)
    High,
    /// Active low (inverted)
    Low,
}

impl Default for ActiveLevel {
    fn default() -> Self {
        Self::High
    }
}

/// Configuration for a single ATX key (power or reset)
/// This is the "four-tuple" configuration: (driver, device, pin/channel, level)
/// For MiIoT driver, uses prop/value instead of device/pin.
/// Power key additionally has off_prop/off_value for force-off (long press).
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AtxKeyConfig {
    /// Driver type (GPIO, USB Relay, or MiIoT)
    pub driver: AtxDriverType,
    /// Device path:
    /// - For GPIO: /dev/gpiochipX
    /// - For USB Relay: /dev/hidrawX
    pub device: String,
    /// Pin or channel number:
    /// - For GPIO: GPIO pin number
    /// - For USB Relay: relay channel (0-based)
    pub pin: u32,
    /// Active level (only applicable to GPIO, ignored for USB Relay)
    pub active_level: ActiveLevel,
    /// MiIoT property name for power-on / action (only for driver=Miot)
    pub prop: String,
    /// MiIoT property value for power-on / action (only for driver=Miot)
    pub value: String,
    /// MiIoT property name for force-off (only for power key with driver=Miot)
    pub off_prop: String,
    /// MiIoT property value for force-off (only for power key with driver=Miot)
    pub off_value: String,
}

impl Default for AtxKeyConfig {
    fn default() -> Self {
        Self {
            driver: AtxDriverType::None,
            device: String::new(),
            pin: 0,
            active_level: ActiveLevel::High,
            prop: String::new(),
            value: String::new(),
            off_prop: String::new(),
            off_value: String::new(),
        }
    }
}

impl AtxKeyConfig {
    /// Check if this key is configured
    pub fn is_configured(&self) -> bool {
        match self.driver {
            AtxDriverType::None => false,
            AtxDriverType::Gpio | AtxDriverType::UsbRelay => !self.device.is_empty(),
            AtxDriverType::Miot => !self.prop.is_empty(),
        }
    }
}

/// MiIoT smart plug connection settings
///
/// Global settings for the MiIoT device (shared by all keys using driver=Miot).
/// The actual prop/value for each action is configured per-key in AtxKeyConfig.
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MiotConfig {
    /// Device ID (DID) for the MiIoT device
    pub did: String,
    /// Path to mijiaAPI command (default: "mijiaAPI")
    pub command: String,
    /// Path to auth/token file for mijiaAPI (optional)
    pub auth_path: String,
}

impl Default for MiotConfig {
    fn default() -> Self {
        Self {
            did: String::new(),
            command: "mijiaAPI".to_string(),
            auth_path: String::new(),
        }
    }
}

impl MiotConfig {
    /// Check if MiIoT connection is configured
    pub fn is_configured(&self) -> bool {
        !self.did.is_empty()
    }
}

/// Driver type for ATX status detection
#[typeshare]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AtxStatusDriverType {
    /// Disabled / Not configured
    None,
    /// LED sensing via GPIO
    Led,
    /// MiIoT smart plug status query
    Miot,
}

impl Default for AtxStatusDriverType {
    fn default() -> Self {
        Self::None
    }
}

/// Status detection configuration
///
/// Determines how to detect the power state of the target machine.
/// - Led: reads a GPIO pin connected to the host power LED
/// - Miot: queries a MiIoT device property
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AtxStatusConfig {
    /// Status detection driver
    pub driver: AtxStatusDriverType,
    /// GPIO chip for LED sensing (driver=Led)
    pub gpio_chip: String,
    /// GPIO pin for LED input (driver=Led)
    pub gpio_pin: u32,
    /// Whether LED is active low / inverted logic (driver=Led)
    pub inverted: bool,
    /// MiIoT property to read for status (driver=Miot)
    pub prop: String,
    /// Value that means "power on" (driver=Miot)
    pub on_value: String,
    /// Value that means "power off" (driver=Miot)
    pub off_value: String,
}

impl Default for AtxStatusConfig {
    fn default() -> Self {
        Self {
            driver: AtxStatusDriverType::None,
            gpio_chip: String::new(),
            gpio_pin: 0,
            inverted: false,
            prop: String::new(),
            on_value: String::new(),
            off_value: String::new(),
        }
    }
}

impl AtxStatusConfig {
    /// Check if status detection is configured
    pub fn is_configured(&self) -> bool {
        match self.driver {
            AtxStatusDriverType::None => false,
            AtxStatusDriverType::Led => !self.gpio_chip.is_empty(),
            AtxStatusDriverType::Miot => !self.prop.is_empty(),
        }
    }
}

/// Internal LED sensing configuration used by LedSensor
/// Constructed from AtxStatusConfig when driver=Led
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AtxLedConfig {
    pub enabled: bool,
    pub gpio_chip: String,
    pub gpio_pin: u32,
    pub inverted: bool,
}

impl Default for AtxLedConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            gpio_chip: String::new(),
            gpio_pin: 0,
            inverted: false,
        }
    }
}

impl AtxLedConfig {
    pub fn is_configured(&self) -> bool {
        self.enabled && !self.gpio_chip.is_empty()
    }
}

/// ATX state information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtxState {
    /// Whether ATX feature is available/enabled
    pub available: bool,
    /// Whether power button is configured
    pub power_configured: bool,
    /// Whether reset button is configured
    pub reset_configured: bool,
    /// Current power status
    pub power_status: PowerStatus,
    /// Whether status detection is supported
    pub status_supported: bool,
}

impl Default for AtxState {
    fn default() -> Self {
        Self {
            available: false,
            power_configured: false,
            reset_configured: false,
            power_status: PowerStatus::Unknown,
            status_supported: false,
        }
    }
}

/// ATX power action request
#[derive(Debug, Clone, Deserialize)]
pub struct AtxPowerRequest {
    /// Action to perform: "short", "long", "reset"
    pub action: AtxAction,
}

/// ATX power action
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AtxAction {
    /// Short press power button (turn on or graceful shutdown)
    Short,
    /// Long press power button (force power off)
    Long,
    /// Press reset button
    Reset,
}

/// Available ATX devices for discovery
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtxDevices {
    /// Available GPIO chips (/dev/gpiochip*)
    pub gpio_chips: Vec<String>,
    /// Available USB HID relay devices (/dev/hidraw*)
    pub usb_relays: Vec<String>,
}

impl Default for AtxDevices {
    fn default() -> Self {
        Self {
            gpio_chips: Vec::new(),
            usb_relays: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_power_status_default() {
        assert_eq!(PowerStatus::default(), PowerStatus::Unknown);
    }

    #[test]
    fn test_atx_driver_type_default() {
        assert_eq!(AtxDriverType::default(), AtxDriverType::None);
    }

    #[test]
    fn test_active_level_default() {
        assert_eq!(ActiveLevel::default(), ActiveLevel::High);
    }

    #[test]
    fn test_atx_key_config_default() {
        let config = AtxKeyConfig::default();
        assert_eq!(config.driver, AtxDriverType::None);
        assert!(config.device.is_empty());
        assert_eq!(config.pin, 0);
        assert!(config.prop.is_empty());
        assert!(config.value.is_empty());
        assert!(config.off_prop.is_empty());
        assert!(config.off_value.is_empty());
        assert!(!config.is_configured());
    }

    #[test]
    fn test_atx_key_config_is_configured() {
        let mut config = AtxKeyConfig::default();
        assert!(!config.is_configured());

        config.driver = AtxDriverType::Gpio;
        assert!(!config.is_configured()); // device still empty

        config.device = "/dev/gpiochip0".to_string();
        assert!(config.is_configured());

        config.driver = AtxDriverType::None;
        assert!(!config.is_configured()); // driver is None
    }

    #[test]
    fn test_atx_key_config_miot_configured() {
        let mut config = AtxKeyConfig::default();
        config.driver = AtxDriverType::Miot;
        assert!(!config.is_configured()); // prop still empty

        config.prop = "on".to_string();
        assert!(config.is_configured());
    }

    #[test]
    fn test_atx_status_config_default() {
        let config = AtxStatusConfig::default();
        assert_eq!(config.driver, AtxStatusDriverType::None);
        assert!(config.gpio_chip.is_empty());
        assert!(!config.is_configured());
    }

    #[test]
    fn test_atx_status_config_led_configured() {
        let mut config = AtxStatusConfig::default();
        assert!(!config.is_configured());

        config.driver = AtxStatusDriverType::Led;
        assert!(!config.is_configured()); // gpio_chip still empty

        config.gpio_chip = "/dev/gpiochip0".to_string();
        assert!(config.is_configured());
    }

    #[test]
    fn test_atx_status_config_miot_configured() {
        let mut config = AtxStatusConfig::default();
        config.driver = AtxStatusDriverType::Miot;
        assert!(!config.is_configured()); // prop still empty

        config.prop = "on".to_string();
        assert!(config.is_configured());
    }

    #[test]
    fn test_atx_state_default() {
        let state = AtxState::default();
        assert!(!state.available);
        assert!(!state.power_configured);
        assert!(!state.reset_configured);
        assert_eq!(state.power_status, PowerStatus::Unknown);
    }

    #[test]
    fn test_miot_config_default() {
        let config = MiotConfig::default();
        assert!(config.did.is_empty());
        assert_eq!(config.command, "mijiaAPI");
        assert!(!config.is_configured());
    }

    #[test]
    fn test_miot_config_is_configured() {
        let mut config = MiotConfig::default();
        assert!(!config.is_configured());

        config.did = "2094828328".to_string();
        assert!(config.is_configured());
    }
}
