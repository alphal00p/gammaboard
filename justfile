restart-db:
    docker-compose down -v
    docker-compose up -d
    sqlx migrate run

serve-backend:
    @echo "Starting Rust API server..."
    cargo run --bin server &
serve-frontend:
    @echo "Starting frontend..."
    cd dashboard && npm start &
serve:
    just serve-backend
    just serve-frontend

stop-backend:
    @echo "Stopping backend..."
    -pkill -f "target/debug/server" || echo "Backend not running"
    @echo "Backend stopped"

stop-frontend:
    @echo "Stopping frontend..."
    -pkill -f "react-scripts" || echo "Frontend not running"
    @echo "Frontend stopped"

stop:
    just stop-backend
    just stop-frontend
