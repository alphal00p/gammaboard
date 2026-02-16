# Gammaboard

Adaptive numerical integration system with distributed computation using PostgreSQL as a work queue.

## Architecture

### Core Components

- **Rust Library** (`src/lib.rs`, `src/batch.rs`) - Database operations and batch abstractions
- **API Server** (`src/bin/server.rs`) - Axum-based REST API serving the dashboard
- **Worker** (planned) - Evaluates batches of samples
- **Frontend** (`dashboard/frontend/`) - React-based visualization dashboard

### Database Schema

**Batch-based Work Queue:**
- `runs` - Integration run metadata and parameters
- `batches` - Work queue containing sample batches (pending → claimed → completed)
- `aggregated_results` - Periodic snapshots of cumulative statistics
- Views: `run_progress`, `work_queue_stats`

## Getting Started

### Prerequisites

- Rust (edition 2024)
- PostgreSQL 16
- Node.js & npm (for frontend)
- Docker (optional, for PostgreSQL)

### Setup

1. **Start PostgreSQL:**
   ```bash
   just restart-db
   ```

2. **Start the API server and frontend:**
   ```bash
   just serve
   ```
   - API: http://localhost:4000/api
   - Frontend: http://localhost:3000

3. **Run test data generator (optional):**
   ```bash
   cargo run --bin test_live_view
   ```

## Usage

### Test the Live Dashboard

```bash
# Terminal 1: Start database
just restart-db

# Terminal 2: Start server + frontend
just serve

# Terminal 3: Generate test data
cargo run --bin test_live_view
```

Visit http://localhost:3000 to see real-time visualization.

### API Endpoints

- `GET /api/health` - Health check
- `GET /api/runs` - List all runs with progress
- `GET /api/runs/:id` - Get specific run details
- `GET /api/runs/:id/stats` - Work queue statistics
- `GET /api/runs/:id/batches?limit=N` - Get completed batches
- `GET /api/runs/:id/samples?limit=N` - Get flattened sample data for plotting

## Development

### Project Structure

```
gammaboard/
├── src/
│   ├── lib.rs              # Database query functions
│   ├── batch.rs            # Batch abstraction types
│   └── bin/
│       ├── server.rs       # API server
│       └── test_live_view.rs  # Test data generator
├── migrations/             # SQL migrations
├── dashboard/
│   ├── frontend/           # React app
│   └── backend/            # (deprecated Node.js backend)
├── Cargo.toml
└── justfile
```

### Database Commands

```bash
# Restart database (drops all data)
just restart-db

# Connect to database
docker exec -it gammaboard psql -U postgres -d gammaboard_db

# View migrations
sqlx migrate info
```

### Key Types

**`Batch`** - Container for weighted sample points
```rust
pub struct Batch {
    pub points: Vec<WeightedPoint>,
}

pub struct WeightedPoint {
    pub point: JsonValue,
    pub weight: f64,
}
```

**`BatchResults`** - Evaluated values
```rust
pub struct BatchResults {
    pub values: Vec<f64>,
}
```

## Workflow

1. **Sampler** generates batches of weighted points → `INSERT INTO batches`
2. **Worker** claims batch → `UPDATE batches ... FOR UPDATE SKIP LOCKED`
3. **Worker** evaluates samples → computes results
4. **Worker** submits results → `UPDATE batches SET status='completed', results=...`
5. **Periodic aggregation** → `INSERT INTO aggregated_results` removes old completed batches
6. **Dashboard** displays live progress

## Next Steps

- [ ] Implement worker binary
- [ ] Python/Rust bridge for adaptive sampler
- [ ] Aggregation functions
- [ ] Distributed worker support
- [ ] Convergence detection

## License

TBD
