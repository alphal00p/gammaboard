# Gammaboard

Adaptive numerical integration system with distributed computation using PostgreSQL as a work queue.

## Architecture

### Core Components

- **Rust Library** (`src/lib.rs`, `src/batch.rs`) - Database operations and batch abstractions used by server, worker, and sampler-aggregator
- **API Server** (`src/bin/server.rs`) - Exposes REST endpoints for runs, batches, workers, and dashboard views; manages run lifecycle and status updates
- **Worker** (planned) - Claims pending batches, evaluates samples, writes batch results/status/errors, and reports worker performance/heartbeats
- **Sampler-Aggregator** - Generates new batches from sampler state, aggregates completed batch results into `aggregated_results`, updates `sampler_states`, and refreshes run summaries
- **Frontend** (`dashboard/frontend/`) - React-based visualization dashboard

### Database Schema

**Batch-based Work Queue:**
- `runs` - Integration run metadata and parameters
- `batches` - Work queue containing sample batches (pending → claimed → completed)
- `aggregated_results` - Periodic snapshots of cumulative statistics
- Views: `run_progress`, `work_queue_stats`


### Prerequisites

- Rust (edition 2024)
- PostgreSQL 16
- Node.js & npm (for frontend)
- Docker (optional, for PostgreSQL)
