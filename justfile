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
    @just serve-backend
    @just serve-frontend

stop-backend:
    -pkill -f "{{justfile_directory()}}/target/debug/server"
    @echo "Backend stopped"

stop-frontend:
    -pkill -f "gammaboard/dashboard.*react-scripts"
    @echo "Frontend stopped"

stop:
    @just stop-backend
    @just stop-frontend
