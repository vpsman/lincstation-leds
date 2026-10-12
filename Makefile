# Build and install on the NAS itself. Run "make install" and "make uninstall" as root.

BIN = target/release/lincstation-leds

build:
	cargo build --release --locked

test:
	cargo test --locked

install: $(BIN)
	install -Dm755 $(BIN) /usr/sbin/lincstation-leds
	install -Dm644 debian/lincstation-leds.service /etc/systemd/system/lincstation-leds.service
	install -Dm644 debian/modules-load.conf /etc/modules-load.d/lincstation-leds.conf
	[ -e /etc/lincstation-leds.conf ] || install -Dm644 debian/lincstation-leds.conf /etc/lincstation-leds.conf
	modprobe i2c-dev
	systemctl daemon-reload
	systemctl enable lincstation-leds
	systemctl restart lincstation-leds

uninstall:
	-systemctl disable --now lincstation-leds
	rm -f /usr/sbin/lincstation-leds /etc/systemd/system/lincstation-leds.service \
		/etc/modules-load.d/lincstation-leds.conf
	systemctl daemon-reload
	@echo "/etc/lincstation-leds.conf kept"

$(BIN):
	@echo "run 'make' first" >&2; exit 1

.PHONY: build test install uninstall
