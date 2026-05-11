set shell := ["bash", "-uc"]

default:
    @just --list

build:
    cargo build

release:
    cargo build --release

run *args:
    cargo run -- {{args}}

check:
    cargo clippy
    cargo fmt --check

fmt:
    cargo fmt

clean:
    cargo clean

install: release
    @cp target/release/grim ~/.local/bin/grim
    @if [ "$(uname)" = "Darwin" ]; then codesign -s - ~/.local/bin/grim; fi
    @echo "Installed → ~/.local/bin/grim"
