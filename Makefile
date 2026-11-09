build: target/release/rorg
target/release/rorg:
	cargo build --release
install: build
	mv target/release/rorg ~/.local/bin/rorg
