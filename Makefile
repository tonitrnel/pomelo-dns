.ONESHELL:

VERSION := 0.1.0

build: build-image
	docker save -o ./pomelo.img pomelo:$(VERSION)

build-image:
	docker build -t pomelo:$(VERSION) .