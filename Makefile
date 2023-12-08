gen_tls:
	openssl req -new \
		-config tests/fixtures/tls/openssl.conf \
		-newkey rsa:4096 \
		-x509 -sha256 -days 9999 -nodes \
		-out ./tests/fixtures/tls/fake.crt \
		-keyout ./tests/fixtures/tls/fake.key
