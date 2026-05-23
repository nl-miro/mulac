COMPOSE_FILE ?= docker-compose.yml
COMPOSE_PROJECT_NAME ?= test_apps

.PHONY: up down reset logs ps help

up:
	@echo "Starting test apps infrastructure (postgres + rabbitmq)..."
	docker compose -f $(COMPOSE_FILE) -p $(COMPOSE_PROJECT_NAME) up -d
	@echo "Services started. Waiting for health checks..."
	docker compose -f $(COMPOSE_FILE) -p $(COMPOSE_PROJECT_NAME) ps

down:
	@echo "Stopping test apps infrastructure..."
	docker compose -f $(COMPOSE_FILE) -p $(COMPOSE_PROJECT_NAME) down
	@echo "Services stopped."

reset:
	@echo "Resetting test apps infrastructure (removing volumes)..."
	docker compose -f $(COMPOSE_FILE) -p $(COMPOSE_PROJECT_NAME) down -v
	docker compose -f $(COMPOSE_FILE) -p $(COMPOSE_PROJECT_NAME) up -d
	@echo "Infrastructure reset and restarted."

logs:
	docker compose -f $(COMPOSE_FILE) -p $(COMPOSE_PROJECT_NAME) logs -f

ps:
	docker compose -f $(COMPOSE_FILE) -p $(COMPOSE_PROJECT_NAME) ps

help:
	@echo "Test Apps Docker Makefile"
	@echo ""
	@echo "Usage: make [target]"
	@echo ""
	@echo "Targets:"
	@echo "  up       - Start all test app services (postgres + rabbitmq for todo & twitter)"
	@echo "  down     - Stop all test app services"
	@echo "  reset    - Reset all services and clear volumes"
	@echo "  logs     - Show logs from all services"
	@echo "  ps       - Show status of all services"
	@echo "  help     - Show this help message"
	@echo ""
	@echo "Individual app targets (run from todo/ or twitter/ directories):"
	@echo "  make up      - Start services for that app"
	@echo "  make down    - Stop services for that app"
	@echo "  make reset   - Reset services for that app"
	@echo "  make test    - Run tests for that app"
	@echo "  make serve   - Run the app server"
	@echo "  make migrate - Run database migrations"
