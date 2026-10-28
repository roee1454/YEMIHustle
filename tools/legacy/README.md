# Legacy Python tools

These produced the character data before the converter was rewritten in Rust (`crates/convert`).
They are kept only as the reference the Rust crate was checked against (`crates/convert/tests/
parity.rs` compares the two outputs byte for byte, using files these tools wrote). The game no
longer runs them; the setup wizard converts characters with the crate, about 70 times faster.
