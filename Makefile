deps:
	go install filippo.io/mkcert@latest

gen_tls:
	mkcert -cert-file tests/fixtures/tls/server.pem -key-file tests/fixtures/tls/server.key localhost 127.0.0.1 ::1

# Trust local CA for testing purposes.
local_cert_trust:
	mkcert -install
	@echo "Ensure you run 'make local_cert_untrust' once you are done testing."

local_cert_untrust:
	mkcert -uninstall
