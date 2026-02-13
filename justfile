restart-db:
    docker-compose down -v
    docker-compose up -d
    sqlx migrate run

serve:
    @echo "Starting backend..."
    cd dashboard/backend && node index.js &

    @echo "Starting frontend..."
    cd dashboard/frontend && npm start
