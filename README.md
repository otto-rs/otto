# otto
otto program for make-like task mgmt via yaml file

## Installation

### GitHub Actions (Recommended for CI/CD)

Use [setup-otto](https://github.com/scottidler/setup-otto) to install otto in your workflows:

```yaml
- uses: scottidler/setup-otto@v1

- name: Run otto tasks
  run: otto ci
```

See [setup-otto](https://github.com/scottidler/setup-otto) for full documentation and options.

### Manual Installation

```bash
# Download the latest release (Linux)
curl -L https://github.com/scottidler/otto/releases/latest/download/otto-vX.Y.Z-linux.tar.gz | tar -xz
sudo mv otto /usr/local/bin/

# macOS (Apple Silicon)
curl -L https://github.com/scottidler/otto/releases/latest/download/otto-vX.Y.Z-macos-arm64.tar.gz | tar -xz
sudo mv otto /usr/local/bin/

# macOS (Intel)
curl -L https://github.com/scottidler/otto/releases/latest/download/otto-vX.Y.Z-macos-x86_64.tar.gz | tar -xz
sudo mv otto /usr/local/bin/
```

### From Source

```bash
cargo install --git https://github.com/scottidler/otto
```

## Version Reporting

The `otto` binary supports `--version` and `-v` flags:

```
$ otto --version
otto v0.1.0-3-gabcdef
```

- The version is driven by the latest annotated git tag and the output of `git describe`.
- If the current commit is exactly at a tag (e.g., `v0.1.0`), the version will be `otto v0.1.0`.
- If there are additional commits, it will show something like `otto v0.1.0-3-gabcdef`.

## Release & Versioning Process

1. **Bump the version in `Cargo.toml`** to the new release version (e.g., `0.2.0`).
2. **Commit** the change.
3. **Tag** the commit with an annotated tag: `git tag -a v0.2.0 -m "Release v0.2.0"`.
4. **Push** the tag: `git push --tags`.
5. **Build** the binary. The version will be embedded from the tag and `git describe`.
6. **Create a GitHub Release** and upload the binary. The version in the binary will match the release tag.

> If the version in `Cargo.toml` does not match the latest tag, a warning will be printed at build time.
