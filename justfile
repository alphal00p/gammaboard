


docker-compose-up:
    docker-compose up -d --force-recreate
# starts all services in docker\docker-compose.yml

docker-compose-down:
    docker-compose down
# stops all services

docker-exec-psql:
    docker exec -it gammaboard psql -U postgres
# run a command in interactive terminal session
# psql -U postgres

docker-list-all:
    docker ps -a


restart-db:
    docker-compose down -v
    docker-compose up -d
    sqlx migrate run

# Justfile

# Run both backend and frontend
# Justfile (run from project root)

serve:
    @echo "Starting backend..."
    cd dashboard/backend && node index.js &

    @echo "Starting frontend..."
    cd dashboard/frontend && npm start

kill:
    @echo "Killing backend..."
    @pkill -f "node index.js" 2>/dev/null || true

    @echo "Killing frontend..."
    @pkill -f "react-scripts" 2>/dev/null || true


