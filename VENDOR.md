# Vendor Dependencies

The `vendor/` directory is not committed to this repository.

To restore vendored Cargo dependencies, run:

```bash
cargo vendor
```

This will populate the `vendor/` directory and update `.cargo/config.toml` automatically.
