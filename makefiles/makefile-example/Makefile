PROJECT         := github.com/dmswe/ttha
GOCMD           := go
GOBUILD         := $(GOCMD) build
GOINSTALL       := $(GOCMD) install
GOCLEAN         := $(GOCMD) clean
GOTEST          := $(GOCMD) test
GOGET           := $(GOCMD) get
GOLINT          := golint
BINARY_NAME     := ttha
GO_FILES        := $(shell find . -name '*.go' | grep -v _test.go)
NAME            := $(PROJECT)
GIT_COMMIT      ?= $(shell git rev-parse --short HEAD)

.PHONY: help clean lint build-linux build-mac dep

# HELP
help: ## Help me.
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

test:
	$(GOTEST) ./... -v

clean: ## Clean Project
	@echo "==> Clean project..."
	$(GOCLEAN)
	rm -f $(BINARY_NAME)
	rm -f $(BINARY_NAME).tar.gz

lint: ## Lint Project
	@echo "==> Lint project..."
	$(GOLINT) ./...

# Cross compilation
build-linux: dep ## Build linux artifact
	@echo "==> Build Linux artifact..."
	CGO_ENABLED=0 GOOS=linux GOARCH=amd64 $(GOBUILD) -o $(BINARY_NAME) -v

build-mac: dep ## Build linux artifact
	@echo "==> Build OSX artifact..."
	CGO_ENABLED=0 GOOS=darwin GOARCH=amd64 $(GOBUILD) -o $(BINARY_NAME) -v

package: build ## Package source code
	@echo "==> Package source code..."
	tar -czvf $(PROJECT_NAME).tar.gz $(BINARY_NAME)

build-docker: ## Build docker image with buildkit
	@echo "==> Package source code..."
	DOCKER_BUILDKIT=1 docker build -t ttha .

# helpers
dep: ## Install deps
	@echo "==> Installing dependencies..."
	$(GOGET) -v -d ./...
	$(GOGET) -u golang.org/x/lint/golint

fmt:
	@echo "==> Fixing source code with gofmt..."
	gofmt -s -w ./main.go ./meta.go ./cmd