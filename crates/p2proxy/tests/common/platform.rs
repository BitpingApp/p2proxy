//! Platform-specific test configuration and helpers
//!
//! This module provides utilities for handling platform-specific differences
//! in test execution, particularly for macOS vs Linux timing differences.

use std::time::Duration;

// Platform multiplier constants with rationale
//
// These multipliers account for platform-specific scheduling and timing overhead:
// - macOS: Higher overhead due to Mach kernel scheduling (1x = full latency)
// - Linux: Lower overhead with efficient scheduler (0.5x = half latency)
// - Windows: Conservative default similar to macOS (1x = full latency)

/// macOS uses full base latency due to higher scheduler overhead
const MACOS_LATENCY_MULTIPLIER: f64 = 1.0;

/// Linux can use reduced latency (50% of base) due to efficient scheduler
const LINUX_LATENCY_DIVISOR: u64 = 2;

/// Windows uses full base latency (conservative default)
const WINDOWS_LATENCY_MULTIPLIER: f64 = 1.0;

/// Minimum latency in milliseconds to prevent zero-duration operations
const MIN_LATENCY_MS: u64 = 1;

/// macOS timeout multiplier (2x) to account for slower test execution
const MACOS_TIMEOUT_MULTIPLIER: u32 = 2;

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

/// Returns true if running on Windows
#[inline]
pub fn is_windows() -> bool {
    cfg!(target_os = "windows")
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
/// // On Linux/Windows: 1 second
/// // On macOS: 2 seconds
/// ```
pub fn platform_timeout(base_duration: Duration) -> Duration {
    if is_macos() {
        // macOS tests run slower, apply 2x multiplier
        base_duration * MACOS_TIMEOUT_MULTIPLIER
    } else {
        base_duration
    }
}

/// Get a platform-adjusted latency for mock operations
///
/// Returns reduced latency for test operations to speed up test execution
/// while accounting for platform differences. Ensures a minimum latency
/// to prevent zero-duration operations.
///
/// # Arguments
///
/// * `base_ms` - The base latency in milliseconds
///
/// # Returns
///
/// The adjusted latency as a Duration, with minimum of 1ms
///
/// # Platform Behavior
///
/// - **macOS**: Returns full base latency (1.0x)
/// - **Linux**: Returns half base latency (0.5x) for faster execution
/// - **Windows**: Returns full base latency (1.0x, conservative)
///
/// # Example
///
/// ```no_run
/// use common::platform::platform_latency;
///
/// let latency = platform_latency(10); // 10ms base
/// // On Linux: 5ms
/// // On macOS/Windows: 10ms
/// ```
pub fn platform_latency(base_ms: u64) -> Duration {
    let latency_ms = if is_linux() {
        // Linux can use reduced latency for faster execution
        (base_ms / LINUX_LATENCY_DIVISOR).max(MIN_LATENCY_MS)
    } else if is_macos() {
        // macOS needs full latency due to higher overhead
        base_ms.max(MIN_LATENCY_MS)
    } else if is_windows() {
        // Windows uses conservative default (full latency)
        base_ms.max(MIN_LATENCY_MS)
    } else {
        // Other platforms: use conservative default
        base_ms.max(MIN_LATENCY_MS)
    };

    Duration::from_millis(latency_ms)
}

/// Get a platform-adjusted sleep duration for test setup
///
/// This is used for delays between test setup and execution, such as
/// waiting for servers to start. Ensures minimum necessary delays.
///
/// # Arguments
///
/// * `base_ms` - The base sleep duration in milliseconds
///
/// # Returns
///
/// The adjusted sleep duration with platform-specific minimums
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
    } else if is_windows() {
        // Windows also needs conservative defaults
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
    } else if is_windows() {
        "Windows"
    } else {
        "Unknown"
    }
}

/// Latency thresholds for test assertions
///
/// Different platforms have different overhead characteristics, so we use
/// platform-specific thresholds to avoid spurious test failures while
/// maintaining strict performance validation.
#[derive(Debug, Clone, Copy)]
pub struct LatencyThresholds {
    /// Maximum acceptable median latency
    pub max_median: Duration,
    /// Maximum acceptable 95th percentile latency
    pub max_p95: Duration,
    /// Maximum acceptable 99th percentile latency
    pub max_p99: Duration,
}

impl LatencyThresholds {
    /// Create new thresholds with the same value for all percentiles
    pub fn uniform(duration: Duration) -> Self {
        Self {
            max_median: duration,
            max_p95: duration,
            max_p99: duration,
        }
    }
}

/// Get platform-specific thresholds for round-trip time tests
///
/// # Expected Values
///
/// Based on 10ms base latency + 1ms jitter:
/// - **Linux**: 15ms median, 18ms p95, 20ms p99
/// - **macOS**: 20ms median, 25ms p95, 30ms p99 (higher due to scheduler overhead)
/// - **Windows**: 20ms median, 25ms p95, 30ms p99 (conservative)
pub fn rtt_thresholds() -> LatencyThresholds {
    if is_linux() {
        LatencyThresholds {
            max_median: Duration::from_millis(15),
            max_p95: Duration::from_millis(18),
            max_p99: Duration::from_millis(20),
        }
    } else {
        // macOS, Windows, and other platforms use conservative thresholds
        LatencyThresholds {
            max_median: Duration::from_millis(20),
            max_p95: Duration::from_millis(25),
            max_p99: Duration::from_millis(30),
        }
    }
}

/// Get platform-specific thresholds for high-latency RTT tests
///
/// # Expected Values
///
/// Based on 200ms base latency + 1ms jitter:
/// - **Linux**: 210ms median, 215ms p95, 220ms p99
/// - **macOS/Windows**: 220ms median, 230ms p95, 240ms p99
pub fn high_latency_rtt_thresholds() -> LatencyThresholds {
    if is_linux() {
        LatencyThresholds {
            max_median: Duration::from_millis(210),
            max_p95: Duration::from_millis(215),
            max_p99: Duration::from_millis(220),
        }
    } else {
        LatencyThresholds {
            max_median: Duration::from_millis(220),
            max_p95: Duration::from_millis(230),
            max_p99: Duration::from_millis(240),
        }
    }
}

/// Get platform-specific thresholds for connection establishment latency
///
/// # Expected Values
///
/// Connection includes handshake + 2x latency (50ms * 2):
/// - **Linux**: 120ms median, 150ms p95, 180ms p99
/// - **macOS/Windows**: 150ms median, 200ms p95, 250ms p99
pub fn connection_latency_thresholds() -> LatencyThresholds {
    if is_linux() {
        LatencyThresholds {
            max_median: Duration::from_millis(120),
            max_p95: Duration::from_millis(150),
            max_p99: Duration::from_millis(180),
        }
    } else {
        LatencyThresholds {
            max_median: Duration::from_millis(150),
            max_p95: Duration::from_millis(200),
            max_p99: Duration::from_millis(250),
        }
    }
}

/// Get platform-specific thresholds for SOCKS5 handshake latency
///
/// # Expected Values
///
/// SOCKS5 handshake = 2 queries with base latency:
/// - **Linux**: 30ms median, 35ms p95, 40ms p99
/// - **macOS/Windows**: 40ms median, 50ms p95, 60ms p99
pub fn socks5_handshake_thresholds() -> LatencyThresholds {
    if is_linux() {
        LatencyThresholds {
            max_median: Duration::from_millis(30),
            max_p95: Duration::from_millis(35),
            max_p99: Duration::from_millis(40),
        }
    } else {
        LatencyThresholds {
            max_median: Duration::from_millis(40),
            max_p95: Duration::from_millis(50),
            max_p99: Duration::from_millis(60),
        }
    }
}

/// Get platform-specific thresholds for first-byte latency
///
/// # Expected Values
///
/// First byte = connection + processing + transfer:
/// - **Linux**: 65ms median, 80ms p95, 100ms p99
/// - **macOS/Windows**: 80ms median, 100ms p95, 120ms p99
pub fn first_byte_latency_thresholds() -> LatencyThresholds {
    if is_linux() {
        LatencyThresholds {
            max_median: Duration::from_millis(65),
            max_p95: Duration::from_millis(80),
            max_p99: Duration::from_millis(100),
        }
    } else {
        LatencyThresholds {
            max_median: Duration::from_millis(80),
            max_p95: Duration::from_millis(100),
            max_p99: Duration::from_millis(120),
        }
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
