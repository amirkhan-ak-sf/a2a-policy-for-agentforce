TARGET                	:= wasm32-wasip1
TARGET_DIR            	:= target/$(TARGET)/release
CARGO_ANYPOINT        	:= cargo-anypoint
POLICY_REF_NAME_SUFFIX 	:= -impl
DEFINITION_NAME        	= $(shell anypoint-cli-v4 pdk policy-project definition get gcl-metadata-name)
DEFINITION_NAMESPACE   	= $(shell anypoint-cli-v4 pdk policy-project definition get gcl-metadata-namespace)
DEFINITION_SRC_GCL_PATH = $(shell anypoint-cli-v4 pdk policy-project locate-gcl definition-src)
DEFINITION_GCL_PATH    	= $(shell anypoint-cli-v4 pdk policy-project locate-gcl definition)
CRATE_NAME             	= $(shell cargo anypoint get-name)
OAUTH_TOKEN            	= $(shell anypoint-cli-v4 pdk get-token)
POLICY_REF_NAME        	= $(DEFINITION_NAME)$(POLICY_REF_NAME_SUFFIX)
SETUP_ERROR_CMD        	= (echo "ERROR:\n\tMissing custom policy project setup. Please run 'make setup'\n")

ifeq ($(OS), Windows_NT)
    SHELL = powershell.exe
    .SHELLFLAGS = -NoProfile -ExecutionPolicy Bypass -Command
	ifneq ($(shell make -v | FIND "GNU Make 4"),)
		ANYPOINT_METADATA_JSON  = $(shell cargo anypoint get-anypoint-metadata | ConvertTo-Json)
	else
		ANYPOINT_METADATA_JSON  = $(shell cargo anypoint get-anypoint-metadata)
	endif
else
	ANYPOINT_METADATA_JSON  = $(shell cargo anypoint get-anypoint-metadata)
endif

.PHONY: setup
setup: install-cargo-anypoint ## Setup all required tools to build
	cargo fetch

.PHONY: build
build: build-asset-files ## Build the policy definition and implementation
	@cargo build --target $(TARGET) --release
	@SRC="$(DEFINITION_GCL_PATH)"; \
		if [ ! -f "$$SRC" ]; then SRC="definition/target/definition/gcl.yaml"; fi; \
		if [ ! -f "$$SRC" ]; then SRC="definition/target/gcl.yaml"; fi; \
		if [ ! -f "$$SRC" ]; then echo "ERROR: cannot locate generated definition gcl.yaml; tried '$(DEFINITION_GCL_PATH)', 'definition/target/definition/gcl.yaml', 'definition/target/gcl.yaml'" >&2; exit 1; fi; \
		cp "$$SRC" "$(TARGET_DIR)/$(CRATE_NAME)_definition.yaml"
	@cargo anypoint gcl-gen -d $(DEFINITION_NAME) -n $(DEFINITION_NAMESPACE) -w $(TARGET_DIR)/$(CRATE_NAME).wasm -o $(TARGET_DIR)/$(CRATE_NAME)_implementation.yaml
	@echo $(POLICY_REF_NAME) > target/policy-ref-name.txt

.PHONY: run
run: build ## Run the policy in local flex
	@anypoint-cli-v4 pdk log -t "warn" -m "Remember to update the config values in playground/config/api.yaml file for the policy configuration"
	@anypoint-cli-v4 pdk patch-gcl -f playground/config/api.yaml -p "spec.policies[0].policyRef.name" -v "$(POLICY_REF_NAME)"
	@anypoint-cli-v4 pdk patch-gcl -f playground/config/api.yaml -p "spec.policies[0].policyRef.namespace" -v "$(DEFINITION_NAMESPACE)"
ifeq ($(OS), Windows_NT)
	rm -Force playground/config/custom-policies/*.yaml
else
	rm -f playground/config/custom-policies/*.yaml
endif
	cp "$(TARGET_DIR)/$(CRATE_NAME)_implementation.yaml" "playground/config/custom-policies/$(CRATE_NAME)_implementation.yaml"
	cp "$(TARGET_DIR)/$(CRATE_NAME)_definition.yaml" "playground/config/custom-policies/$(CRATE_NAME)_definition.yaml"
	-docker compose -f ./playground/docker-compose.yaml down
	docker compose -f ./playground/docker-compose.yaml up

.PHONY: test
test: build ## Run unit tests
	@cargo test -- --nocapture

.PHONY: publish
publish: build ## Publish a development version of the policy
	anypoint-cli-v4 pdk policy-project publish --binary-path $(TARGET_DIR)/$(CRATE_NAME).wasm --implementation-gcl-path $(TARGET_DIR)/$(CRATE_NAME)_implementation.yaml

.PHONY: release
release: build ## Publish a release version
	anypoint-cli-v4 pdk policy-project release --binary-path $(TARGET_DIR)/$(CRATE_NAME).wasm --implementation-gcl-path $(TARGET_DIR)/$(CRATE_NAME)_implementation.yaml

GROUP_ID                = $(shell cargo anypoint get-group-id)
ASSET_VERSION           = $(shell cargo anypoint get-version)
ASSET_ID                = $(shell echo "$(CRATE_NAME)" | tr _ -)
DEFINITION_GAV          = $(GROUP_ID)/$(ASSET_ID)/$(ASSET_VERSION)
HOME_MD                 = definition/home.md

.PHONY: upload-docs
upload-docs: ## Upload definition/home.md as the Exchange home page for the release definition asset
	@if [ ! -f "$(HOME_MD)" ]; then echo "ERROR: $(HOME_MD) is missing"; exit 1; fi
	anypoint-cli-v4 exchange asset page upload -f "$(DEFINITION_GAV)" home "$(HOME_MD)"

.PHONY: upload-docs-dev
upload-docs-dev: ## Upload definition/home.md to a dev definition asset. Pass DEV_VERSION=1.0.1-20260506192149 (from the make publish output)
	@if [ ! -f "$(HOME_MD)" ]; then echo "ERROR: $(HOME_MD) is missing"; exit 1; fi
	@if [ -z "$(DEV_VERSION)" ]; then echo "ERROR: pass DEV_VERSION=<dev asset version>, e.g. make upload-docs-dev DEV_VERSION=1.0.1-20260506192149"; exit 1; fi
	anypoint-cli-v4 exchange asset page upload -f "$(GROUP_ID)/$(ASSET_ID)-dev/$(DEV_VERSION)" home "$(HOME_MD)"

.PHONY: build-asset-files
build-asset-files: $(DEFINITION_SRC_GCL_PATH)
	@anypoint-cli-v4 pdk policy-project build-asset-files --metadata '$(ANYPOINT_METADATA_JSON)'
	@if [ -d definition/target/definition ]; then \
		cp definition/target/definition/gcl.yaml      definition/target/gcl.yaml      2>/dev/null || true; \
		cp definition/target/definition/metadata.yaml definition/target/metadata.yaml 2>/dev/null || true; \
		cp definition/target/definition/exchange.json definition/target/exchange.json 2>/dev/null || true; \
		cp definition/target/definition/schema.json   definition/target/schema.json   2>/dev/null || true; \
	fi
	@if [ -f definition/target/gcl.yaml ] && [ ! -f definition/target/definition/gcl.yaml ]; then \
		mkdir -p definition/target/definition; \
		cp definition/target/gcl.yaml      definition/target/definition/gcl.yaml      2>/dev/null || true; \
		cp definition/target/metadata.yaml definition/target/definition/metadata.yaml 2>/dev/null || true; \
		cp definition/target/exchange.json definition/target/definition/exchange.json 2>/dev/null || true; \
		cp definition/target/schema.json   definition/target/definition/schema.json   2>/dev/null || true; \
	fi
	@cargo anypoint config-gen -p -m $(DEFINITION_SRC_GCL_PATH) -o src/generated/config.rs

.PHONY: login
login:
	@cargo login $(OAUTH_TOKEN)

.PHONY: install-cargo-anypoint
install-cargo-anypoint:
	cargo install cargo-anypoint@1.8.0

.PHONY: show-policy-ref-name
show-policy-ref-name:
	@echo $(POLICY_REF_NAME)

ifneq ($(OS), Windows_NT)
all: help

.PHONY: help
help: ## Shows this help
	@echo 'Usage: make <target>'
	@echo ''
	@echo 'Available targets are:'
	@echo ''
	@grep -Eh '^\w[^:]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-6s\033[0m %s\n", $$1, $$2}' \
		| sort
endif
