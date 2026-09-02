---
name: package-release
description: Package macOS and Linux release binaries into clean tarballs without macOS extended attribute pollution. Use when packaging release assets, building release archives, bundling binaries for GitHub releases, or requested to "打包", "package binary", "create release tarball".
argument-hint: "[target] [version]"
---

# Package Release

Package compiled CLI binaries and essential assets into clean, deterministic release tarballs across macOS and Linux targets.

## Invariant: Clean archive creation

Every `tar` command creating release archives on macOS **MUST** be prefixed with `COPYFILE_DISABLE=1` and exclude `.DS_Store` to prevent Apple extended attributes (`._*` shadow files and `LIBARCHIVE.xattr.*` headers) from leaking into Linux extractions.

```sh
COPYFILE_DISABLE=1 tar --exclude='.DS_Store' -czf "${ARCHIVE_NAME}.tar.gz" -C "${STAGE_PARENT}" "${FOLDER_NAME}"
```

---

## Packaging Steps

Execute these steps in order. Each step ends with an explicit check.

### Step 1: Resolve Target and Version

1. Detect or extract the binary name and current crate version from `Cargo.toml` (e.g. `name = "mena"`, `version = "0.1.0"`).
2. Match the requested platform target triplet:
   - `aarch64-apple-darwin` (macOS Apple Silicon)
   - `x86_64-apple-darwin` (macOS Intel)
   - `x86_64-unknown-linux-gnu` (Linux x86_64)
   - `aarch64-unknown-linux-gnu` (Linux ARM64)
3. Set naming convention:
   - Folder name: `<binary>-v<version>-<target>` (e.g. `mena-v0.1.0-x86_64-unknown-linux-gnu`)
   - Archive name: `<binary>-v<version>-<target>.tar.gz`

**Completion criterion**: Binary name, version tag (`vX.Y.Z`), target triplet, folder name, and archive name are unambiguously determined.

---

### Step 2: Locate or Build the Release Binary

1. Locate the compiled executable:
   - Explicit target build: `target/<target>/release/<binary>`
   - Native host build: `target/release/<binary>`
2. If absent or stale, build it:
   - Native: `cargo build --release --locked`
   - Cross-target (macOS to Linux): `cargo zigbuild --release --target <target> --locked` or `cross build --release --target <target> --locked`
3. Verify executable exists and is non-empty.

**Completion criterion**: Release binary exists at the source path and has executable permissions.

---

### Step 3: Stage Directory Structure

Never create tarballs from bare files or root directories. Stage files into a clean temporary directory first:

1. Create temporary staging directory:
   ```sh
   STAGE_ROOT="$(mktemp -d)"
   STAGE_DIR="${STAGE_ROOT}/${FOLDER_NAME}"
   mkdir -p "${STAGE_DIR}"
   ```
2. Copy the binary and essential documentation:
   ```sh
   cp "${BIN_PATH}" "${STAGE_DIR}/${BINARY_NAME}"
   chmod +x "${STAGE_DIR}/${BINARY_NAME}"
   [ -f README.md ] && cp README.md "${STAGE_DIR}/"
   [ -f LICENSE ] && cp LICENSE "${STAGE_DIR}/"
   [ -f LICENSE-MIT ] && cp LICENSE-MIT "${STAGE_DIR}/"
   [ -f LICENSE-APACHE ] && cp LICENSE-APACHE "${STAGE_DIR}/"
   ```

**Completion criterion**: Staged folder contains only the executable and documentation files; no `.DS_Store`, build artifacts, or dotfiles present.

---

### Step 4: Create Archive with Attribute Protection

1. Execute the packaging command with macOS metadata suppression:
   ```sh
   COPYFILE_DISABLE=1 tar --exclude='.DS_Store' -czf "${ARCHIVE_NAME}" -C "${STAGE_ROOT}" "${FOLDER_NAME}"
   ```
2. Move the generated archive to the project root or destination directory.
3. Clean up the temporary staging directory.

**Completion criterion**: Archive `${ARCHIVE_NAME}` exists in destination and size is > 0 bytes.

---

### Step 5: Verify Archive Purity

Inspect the archive table of contents to verify zero macOS metadata contamination:

```sh
tar -tf "${ARCHIVE_NAME}"
```

Checklist:
- Top-level path starts with `${FOLDER_NAME}/` (no leading `./` or absolute `/` path).
- No `._*` AppleDouble resource fork files.
- No `.DS_Store` files.
- Binary is present and permissions are executable.

**Completion criterion**: `tar -tf` output contains strictly the staged directory and approved files with no `._` entries.

---

## Reference Matrix

| Target | Build Tool on macOS | Output Archive |
|---|---|---|
| `aarch64-apple-darwin` | `cargo build --release` (Native on Apple Silicon) | `<name>-v<ver>-aarch64-apple-darwin.tar.gz` |
| `x86_64-apple-darwin` | `cargo build --release --target x86_64-apple-darwin` | `<name>-v<ver>-x86_64-apple-darwin.tar.gz` |
| `x86_64-unknown-linux-gnu` | `cargo zigbuild` or `cross build` | `<name>-v<ver>-x86_64-unknown-linux-gnu.tar.gz` |
| `aarch64-unknown-linux-gnu` | `cargo zigbuild` or `cross build` | `<name>-v<ver>-aarch64-unknown-linux-gnu.tar.gz` |
