// tests/integration/cross_backend/mod.rs - Cross-backend consistency tests

pub mod consistency;

use anyhow::Result;
use garbagetruck::storage::{MemoryStorage, Storage};



use crate::helpers::{skip_if_no_env, test_data::create_test_lease_data};