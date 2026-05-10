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
    cp target/release/carina ~/.local/bin/carina
    codesign -s - ~/.local/bin/carina
    mkdir -p ~/.config/carina/themes
    cp themes/*.toml ~/.config/carina/themes/
    @echo "Installed → ~/.local/bin/carina"
    @echo "Themes   → ~/.config/carina/themes/"
