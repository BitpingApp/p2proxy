// Integration tests for test utilities
mod common;

#[cfg(test)]
mod tests {
    use crate::common::test_utils::*;
    use std::time::Duration;

    #[test]
    fn test_bandwidth_measurement() {
        let measurement = BandwidthMeasurement::new(1_000_000, Duration::from_secs(1));

        assert_eq!(measurement.total_bytes, 1_000_000);
        assert_eq!(measurement.bytes_per_sec, 1_000_000.0);
        assert_eq!(measurement.mbps(), 8.0); // 1MB/s = 8Mbps
    }

    #[test]
    fn test_bandwidth_measurement_zero_duration() {
        let measurement = BandwidthMeasurement::new(1_000_000, Duration::ZERO);

        assert_eq!(measurement.total_bytes, 1_000_000);
        assert_eq!(measurement.bytes_per_sec, 0.0);
        assert_eq!(measurement.mbps(), 0.0);
    }

    #[test]
    fn test_assert_bandwidth_within_success() {
        // Should not panic
        assert_bandwidth_within(95_000, 100_000, 10.0);
        assert_bandwidth_within(105_000, 100_000, 10.0);
        assert_bandwidth_within(100_000, 100_000, 10.0);
    }

    #[test]
    #[should_panic(expected = "outside tolerance range")]
    fn test_assert_bandwidth_within_failure() {
        // Should panic - outside 10% tolerance
        assert_bandwidth_within(85_000, 100_000, 10.0);
    }

    #[tokio::test]
    async fn test_measure_latency() {
        let stats = measure_latency(
            || async {
                tokio::time::sleep(Duration::from_millis(10)).await;
            },
            10,
        )
        .await;

        // Latencies should be around 10ms (with some tolerance for scheduling)
        assert!(stats.min >= Duration::from_millis(9));
        assert!(stats.max <= Duration::from_millis(20));
        assert!(stats.mean >= Duration::from_millis(9));
        assert!(stats.median >= Duration::from_millis(9));
        assert!(stats.p95 >= Duration::from_millis(9));
        assert!(stats.p99 >= Duration::from_millis(9));
    }

    #[tokio::test]
    async fn test_measure_latency_percentiles() {
        // Test with uniform delay
        let stats = measure_latency(
            || async {
                tokio::time::sleep(Duration::from_millis(5)).await;
            },
            10,
        )
        .await;

        // Verify percentiles are reasonable
        assert!(stats.min <= stats.median);
        assert!(stats.median <= stats.p95);
        assert!(stats.p95 <= stats.p99);
        assert!(stats.p99 <= stats.max);

        // All measurements should be similar (around 5ms)
        assert!(stats.min >= Duration::from_millis(4));
        assert!(stats.max <= Duration::from_millis(10));
    }

    #[tokio::test]
    async fn test_measure_bandwidth() {
        let measurement = measure_bandwidth(|| async {
            tokio::time::sleep(Duration::from_millis(100)).await;
        })
        .await;

        // Should take around 100ms
        assert!(measurement.duration >= Duration::from_millis(90));
        assert!(measurement.duration <= Duration::from_millis(150));
    }
}
