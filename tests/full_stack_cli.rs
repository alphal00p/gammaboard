#![allow(clippy::type_complexity)]

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
};
use assert_cmd::Command;
use gammaboard::Domain;
use gammaboard::api::nodes as node_api;
use gammaboard::config::RuntimeConfig;
use gammaboard::sampling::{
    HavanaSamplerParams, LatentBatch, PdfAdaptationImagePersistedOutput, SamplerAggregatorSnapshot,
};
use predicates::prelude::*;
use rand::SeedableRng;
use rand_xoshiro::Xoshiro256StarStar;
use serde_json::{Value as JsonValue, json};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::collections::HashSet;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use symbolica::numerical_integration::{ContinuousGrid, DiscreteGrid, Grid, Sample};
use tempfile::{NamedTempFile, TempDir};
use tokio::process::{Child, Command as TokioCommand};
use tokio::time::{Instant, sleep};
use url::Url;

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[path = "e2e/support.rs"]
mod support;
use support::*;

#[path = "e2e/workflow.rs"]
mod workflow;

#[path = "e2e/failure_policy.rs"]
mod failure_policy;

#[path = "e2e/controls.rs"]
mod controls;

#[path = "e2e/lifecycle.rs"]
mod lifecycle;

#[path = "e2e/controllers.rs"]
mod controllers;

#[path = "e2e/search.rs"]
mod search;

#[path = "e2e/adapters.rs"]
mod adapters;

#[path = "e2e/contracts.rs"]
mod contracts;

#[cfg(unix)]
#[path = "e2e/recovery.rs"]
mod recovery;
