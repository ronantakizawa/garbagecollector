// tests/helpers/assertions.rs - Custom test assertions

use garbagetruck::lease::{Lease, LeaseState, LeaseStats};

/// Assert that two leases are functionally equivalent (ignoring timestamps)
pub fn assert_leases_equivalent(actual: &Lease, expected: &Lease) {
    assert_eq!(actual.lease_id, expected.lease_id);
    assert_eq!(actual.object_id, expected.object_id);
    assert_eq!(actual.object_type, expected.object_type);
    assert_eq!(actual.service_id, expected.service_id);
    assert_eq!(actual.state, expected.state);
    assert_eq!(actual.metadata, expected.metadata);
    assert_eq!(actual.cleanup_config, expected.cleanup_config);
}

/// Assert that a lease is in the expected state
pub fn assert_lease_state(lease: &Lease, expected_state: LeaseState) {
    assert_eq!(
        lease.state, 
        expected_state,
        "Lease {} should be in state {:?}, but was {:?}",
        lease.lease_id,
        expected_state,
        lease.state
    );
}

/// Assert that a lease is active and not expired
pub fn assert_lease_active(lease: &Lease) {
    assert_eq!(lease.state, LeaseState::Active, "Lease should be active");
    assert!(!lease.is_expired(), "Lease should not be expired");
    assert!(lease.is_active(), "Lease should be considered active");
}

/// Assert that a lease is expired
pub fn assert_lease_expired(lease: &Lease) {
    assert!(
        lease.is_expired() || lease.state == LeaseState::Expired,
        "Lease should be expired or in expired state"
    );
}

/// Assert that lease stats contain expected values
pub fn assert_lease_stats(stats: &LeaseStats, expected_total: usize) {
    assert_eq!(
        stats.total_leases, 
        expected_total,
        "Expected {} total leases, got {}",
        expected_total,
        stats.total_leases
    );
    
    // Basic consistency checks
    assert_eq!(
        stats.total_leases,
        stats.active_leases + stats.expired_leases + stats.released_leases,
        "Lease state counts should sum to total"
    );
}

/// Assert that lease stats contain specific service
pub fn assert_stats_contains_service(stats: &LeaseStats, service_id: &str, expected_count: usize) {
    let actual_count = stats.leases_by_service.get(service_id).unwrap_or(&0);
    assert_eq!(
        *actual_count,
        expected_count,
        "Expected {} leases for service {}, got {}",
        expected_count,
        service_id,
        actual_count
    );
}

/// Assert that cleanup calls contain expected lease
pub fn assert_cleanup_called_for_lease(
    cleanup_calls: &[crate::helpers::mock_server::CleanupCall],
    lease_id: &str
) {
    let found = cleanup_calls.iter().any(|call| call.lease_id == lease_id);
    assert!(
        found,
        "Expected cleanup call for lease {}, but none found. Available calls: {:?}",
        lease_id,
        cleanup_calls.iter().map(|c| &c.lease_id).collect::<Vec<_>>()
    );
}

/// Assert that operation completed within expected time
pub fn assert_duration_reasonable(duration: std::time::Duration, max_expected_ms: u64) {
    assert!(
        duration.as_millis() <= max_expected_ms as u128,
        "Operation took {}ms, expected <= {}ms",
        duration.as_millis(),
        max_expected_ms
    );
}