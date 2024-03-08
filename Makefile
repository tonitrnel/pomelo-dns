.ONESHELL:

PKG_VER        := $(shell cat Cargo.toml | grep "^version" | awk '{print $$3}' | sed 's/"//g')
COMMIT_ID      := $(shell git rev-parse --short=9 HEAD)
DOCKER_VERSION := $(shell docker --version | awk '{print $$3}' | sed 's/,//')
BUILD_DATE     := $(shell date '+%Y-%m-%d')
RUSTC_VERSION  := $(shell rustc --version | awk '{print $$2}')

build: build-image
	docker save -o ./pomelo_${PKG_VER}.img pomelo:$(PKG_VER)

build-image:
	docker build \
		--build-arg PKG_VER=$(PKG_VER) \
		--build-arg COMMIT_ID=$(COMMIT_ID) \
		--build-arg DOCKER_VERSION=$(DOCKER_VERSION) \
		--build-arg BUILD_DATE=$(BUILD_DATE) \
		--build-arg RUSTC_VERSION=$(RUSTC_VERSION) \
		-t pomelo:$(PKG_VER) .