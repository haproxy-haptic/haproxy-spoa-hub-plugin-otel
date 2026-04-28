# Releasing

## Prerequisites

- Write access to the repository
- `glab` CLI authenticated, or ability to push tags
- **Generic packages "Reject duplicates" enabled** under
  Settings > Packages and Registries > Generic packages. Without this, a
  re-run of the release pipeline on the same tag silently overwrites the
  bytes uploaded earlier.

## Steps

1. **Update the changelog**: Move items from `## [Unreleased]` to a new version section and update the compare-link references at the bottom of the file:

   ```markdown
   ## [Unreleased]

   ## [0.1.0] - 2026-04-27

   ### Added
   - ...
   ```

   Follow [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) conventions. Use categories: Added, Changed, Deprecated, Removed, Fixed, Security.

2. **Bump the version** in `Cargo.toml`:

   ```toml
   [package]
   version = "0.1.0"
   ```

3. **Commit and merge**: Create a branch, commit the changelog and version bump, merge to `main` via MR.

4. **Tag the release**: From the merged `main`:

   ```bash
   git checkout main && git pull
   git tag v0.1.0
   git push origin v0.1.0
   ```

   Tags must match the pattern `v*` (e.g., `v0.1.0`, `v1.0.0-beta.1`).

5. **CI handles the rest**: The `release` job in `.gitlab-ci.yml` automatically:
   - Cross-compiles the plugin `.so` for `amd64`, `arm64`, and `armv7` (glibc
     2.36) with `cargo auditable` (binaries embed a compressed
     `Cargo.lock`-equivalent so SBOM tools can enumerate Rust deps without
     source access)
   - Generates per-file `.sha256` sidecars and a combined `SHA256SUMS`
   - Cosign keyless-signs every `.so` using the GitLab OIDC id token,
     producing `.cosign.bundle` files
   - Uploads everything to the Generic Package Registry under
     `packages/generic/otel/<tag>/`
   - Extracts the matching changelog section as release notes
   - Creates a GitLab release with all artifacts linked as release assets

## Verifying a release

After CI finishes, verify the release end-to-end from a clean machine:

```sh
make verify-release VERSION=v0.1.1
```

This downloads `SHA256SUMS`, every artifact named in it, and every artifact's
cosign bundle from the package registry, then runs `sha256sum -c` and
`cosign verify-blob` against the project's tag identity.

## Versioning

This project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html):

- **MAJOR**: Breaking changes to the plugin's TOML config schema (`config/schema.json`), to the SPOE message contract (input arguments or output variables), or to the directive vocabulary mirroring HAProxy's OpenTelemetry addon.
- **MINOR**: New tracer/meter/logger pipelines, new exporter knobs, new config keys with defaults, new return variables, additional propagators.
- **PATCH**: Bug fixes, dependency bumps with no behavior change, performance improvements, documentation updates.

Pre-release versions use suffixes: `v0.1.0-alpha.1`, `v0.1.0-beta.1`, `v0.1.0-rc.1`.
