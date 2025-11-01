//! Platform-specific test configuration and helpers
//!
//! This module provides utilities for handling platform-specific differences
//! in test execution, particularly for macOS vs Linux timing differences.

use std::time::Duration;

/// Returns true if running on macOS
#[inline]
pub fn is_macos() -> bool {
    cfg!(target_os = "macos")
}

/// Returns true if running on Linux
#[inline]
pub fn is_linux() -> bool {
    cfg!(target_os = "linux")
}

/// Get a platform-adjusted timeout duration
///
/// macOS tests often run slower due to different threading/scheduling behavior,
/// so we apply a multiplier to timeouts to prevent spurious failures.
///
/// # Arguments
///
/// * `base_duration` - The base timeout duration for Linux
///
/// # Returns
///
/// The adjusted duration (base duration * platform multiplier)
///
/// # Example
///
/// ```no_run
/// use common::platform::platform_timeout;
/// use std::time::Duration;
///
/// let timeout = platform_timeout(Duration::from_secs(1));
/// // On Linux: 1 second
/// // On macOS: 2 seconds
/// ```
pub fn platform_timeout(base_duration: Duration) -> Duration {
    if is_macos() {
        // macOS tests run slower, apply 2x multiplier
        base_duration * 2
    } else {
        base_duration
    }
}

/// Get a platform-adjusted latency for mock operations
///
/// Returns reduced latency for test operations to speed up test execution
/// while accounting for platform differences.
///
/// # Arguments
///
/// * `base_latency` - The base latency in milliseconds
///
/// # Returns
///
/// The adjusted latency as a Duration
///
/// # Example
///
/// ```no_run
/// use common::platform::platform_latency;
///
/// let latency = platform_latency(10); // 10ms base
/// // On Linux: 5ms
/// // On macOS: 10ms
/// ```
pub fn platform_latency(base_ms: u64) -> Duration {
    if is_macos() {
        // macOS needs slightly more time for operations
        Duration::from_millis(base_ms)
    } else {
        // Linux can be more aggressive with reduced latency
        Duration::from_millis(base_ms / 2)
    }
}

/// Get a platform-adjusted sleep duration for test setup
///
/// This is used for delays between test setup and execution, such as
/// waiting for servers to start.
///
/// # Arguments
///
/// * `base_ms` - The base sleep duration in milliseconds
///
/// # Returns
///
/// The adjusted sleep duration
///
/// # Example
///
/// ```no_run
/// use common::platform::platform_sleep;
///
/// let sleep_dur = platform_sleep(10); // 10ms base
/// tokio::time::sleep(sleep_dur).await;
/// ```
pub fn platform_sleep(base_ms: u64) -> Duration {
    if is_macos() {
        // macOS may need slightly more time for server startup
        Duration::from_millis(base_ms.max(10))
    } else {
        // Linux can use minimal delays
        Duration::from_millis(base_ms.max(5))
    }
}

/// Returns the platform name as a string
pub fn platform_name() -> &'static str {
    if is_macos() {
        "macOS"
    } else if is_linux() {
        "Linux"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_detection() {
        let name = platform_name();
        assert!(!name.is_empty());
        println!("Running on: {}", name);
    }

    #[test]
    fn test_platform_timeout() {
        let base = Duration::from_millis(100);
        let adjusted = platform_timeout(base);

        if is_macos() {
            assert_eq!(adjusted, Duration::from_millis(200));
        } else {
            assert_eq!(adjusted, base);
        }
    }

    #[test]
    fn test_platform_latency() {
        let latency = platform_latency(10);

        if is_macos() {
            assert_eq!(latency, Duration::from_millis(10));
        } else {
            assert_eq!(latency, Duration::from_millis(5));
        }
    }

    #[test]
    fn test_platform_sleep() {
        let sleep_dur = platform_sleep(10);

        if is_macos() {
            assert_eq!(sleep_dur, Duration::from_millis(10));
        } else {
            assert_eq!(sleep_dur, Duration::from_millis(5));
        }
    }
}
