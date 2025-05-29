.PHONY: help build test clean docker-build docker-run dev-up dev-down proto lint fmt check

# Default target
help: ## Show this help message
	@echo "Available targets:"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  %-15s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

# Build targets
build: ## Build the application in release mode
	cargo build --release

build-dev: ## Build the application in debug mode
	cargo build

# Testing targets
test: ## Run all tests
	cargo test

test-integration: ## Run integration tests
	cargo test --features integration

bench: ## Run benchmarks
	cargo bench

# Code quality targets
lint: ## Run clippy linter
	cargo clippy -- -D warnings

fmt: ## Format code
	cargo fmt

fmt-check: ## Check code formatting
	cargo fmt -- --check

check: ## Check code without building
	cargo check

# Protocol buffer generation
proto: ## Generate protobuf code
	cargo build

proto-clean: ## Clean generated protobuf files
	rm -rf src/generated/

# Docker targets
docker-build: ## Build Docker image
	docker build -t distributed-gc-sidecar:latest .

docker-build-dev: ## Build Docker image for development
	docker build -t distributed-gc-sidecar:dev .

docker-run: ## Run Docker container
	docker run -p 50051:50051 -p 9090:9090 distributed-gc-sidecar:latest

# Development environment
dev-up: ## Start development environment
	docker-compose up -d

dev-down: ## Stop development environment
	docker-compose down

dev-logs: ## Show development environment logs
	docker-compose logs -f

dev-restart: ## Restart development environment
	docker-compose restart gc-sidecar

# Database management
db-migrate: ## Run database migrations
	sqlx migrate run --database-url "postgresql://gcuser:gcpass@localhost:5432/distributed_gc"

db-reset: ## Reset database
	docker-compose exec postgres psql -U gcuser -d distributed_gc -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"
	$(MAKE) db-migrate

db-shell: ## Connect to database shell
	docker-compose exec postgres psql -U gcuser -d distributed_gc

# Monitoring and metrics
metrics: ## Show current metrics
	curl -s http://localhost:9090/metrics | grep gc_

health: ## Check service health
	grpcurl -plaintext localhost:50051 distributed_gc.DistributedGCService/HealthCheck

# Load testing
load-test: ## Run load tests
	./scripts/load-test.sh

stress-test: ## Run stress tests
	./scripts/stress-test.sh

# Cleanup targets
clean: ## Clean build artifacts
	cargo clean
	docker-compose down -v
	docker system prune -f

clean-all: ## Clean everything including Docker images
	$(MAKE) clean
	docker rmi distributed-gc-sidecar:latest distributed-gc-sidecar:dev 2>/dev/null || true

# Installation targets
install-deps: ## Install development dependencies
	# Install Rust toolchain
	rustup update stable
	rustup component add clippy rustfmt
	
	# Install protobuf compiler
	@echo "Installing protobuf compiler..."
	@if command -v apt-get > /dev/null; then \
		sudo apt-get update && sudo apt-get install -y protobuf-compiler; \
	elif command -v brew > /dev/null; then \
		brew install protobuf; \
	elif command -v yum > /dev/null; then \
		sudo yum install -y protobuf-compiler; \
	else \
		echo "Please install protobuf compiler manually"; \
	fi
	
	# Install grpcurl for testing
	go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest

install-k8s-deps: ## Install Kubernetes dependencies
	# Install helm
	curl https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 | bash
	
	# Install kubectl (if not present)
	@if ! command -v kubectl > /dev/null; then \
		curl -LO "https://dl.k8s.io/release/$$(curl -L -s https://dl.k8s.io/release/stable.txt)/bin/linux/amd64/kubectl"; \
		chmod +x kubectl; \
		sudo mv kubectl /usr/local/bin/; \
	fi

# Release targets
release-prepare: ## Prepare release
	$(MAKE) fmt
	$(MAKE) lint
	$(MAKE) test
	$(MAKE) build

release-docker: ## Build and tag Docker image for release
	docker build -t distributed-gc-sidecar:$$(git describe --tags --always) .
	docker tag distributed-gc-sidecar:$$(git describe --tags --always) distributed-gc-sidecar:latest

# Documentation targets
docs: ## Generate documentation
	cargo doc --no-deps --open

docs-api: ## Generate API documentation
	@echo "Generating API documentation from protobuf files..."
	protoc --doc_out=docs --doc_opt=html,api.html proto/*.proto

# Performance profiling
profile: ## Run performance profiling
	cargo build --release
	perf record --call-graph=dwarf ./target/release/gc-sidecar &
	sleep 60
	pkill gc-sidecar
	perf report

flamegraph: ## Generate flamegraph
	cargo flamegraph --bin gc-sidecar

# Security scanning
security-audit: ## Run security audit
	cargo audit

# Kubernetes targets
k8s-deploy: ## Deploy to Kubernetes
	kubectl apply -f k8s/

k8s-delete: ## Delete from Kubernetes
	kubectl delete -f k8s/

k8s-logs: ## Show Kubernetes logs
	kubectl logs -l app=gc-sidecar -f

helm-install: ## Install with Helm
	helm install gc-sidecar ./helm/gc-sidecar

helm-upgrade: ## Upgrade with Helm
	helm upgrade gc-sidecar ./helm/gc-sidecar

helm-uninstall: ## Uninstall with Helm
	helm uninstall gc-sidecar

# Monitoring targets
grafana-import: ## Import Grafana dashboards
	curl -X POST \
		-H "Content-Type: application/json" \
		-d @grafana/dashboards/gc-overview.json \
		http://admin:admin@localhost:3000/api/dashboards/db

prometheus-config: ## Validate Prometheus configuration
	docker run --rm -v $$(pwd)/prometheus.yml:/etc/prometheus/prometheus.yml \
		prom/prometheus:latest promtool check config /etc/prometheus/prometheus.yml

# Development helpers
watch: ## Watch for changes and rebuild
	cargo watch -x build

watch-test: ## Watch for changes and run tests
	cargo watch -x test

tail-logs: ## Tail application logs
	docker-compose logs -f gc-sidecar

debug: ## Run in debug mode
	RUST_LOG=debug cargo run

# Example client runs
example-rust: ## Run Rust client example
	cd examples/rust-client && cargo run

example-python: ## Run Python client example
	cd examples/python-client && python client.py

# Backup and restore
backup-data: ## Backup database
	docker-compose exec postgres pg_dump -U gcuser distributed_gc > backup_$$(date +%Y%m%d_%H%M%S).sql

restore-data: ## Restore database (requires BACKUP_FILE variable)
	@if [ -z "$(BACKUP_FILE)" ]; then \
		echo "Usage: make restore-data BACKUP_FILE=backup_file.sql"; \
		exit 1; \
	fi
	docker-compose exec -T postgres psql -U gcuser distributed_gc < $(BACKUP_FILE)

# Environment setup
setup-dev: install-deps dev-up db-migrate ## Complete development setup
	@echo "Development environment is ready!"
	@echo "- gRPC server: localhost:50051"
	@echo "- Metrics: http://localhost:9090/metrics"
	@echo "- Grafana: http://localhost:3000 (admin/admin)"
	@echo "- PostgreSQL: localhost:5432"

# CI/CD helpers
ci-test: ## Run CI tests
	$(MAKE) fmt-check
	$(MAKE) lint
	$(MAKE) test
	$(MAKE) build

ci-docker: ## Build Docker image for CI
	docker build --target builder -t gc-sidecar-builder .
	docker build -t distributed-gc-sidecar:ci .

# Version information
version: ## Show version information
	@echo "Cargo version: $$(cargo --version)"
	@echo "Rust version: $$(rustc --version)"
	@echo "Project version: $$(grep '^version' Cargo.toml | cut -d'"' -f2)"
	@echo "Git commit: $$(git rev-parse --short HEAD)"
	@echo "Git tag: $$(git describe --tags --always)"