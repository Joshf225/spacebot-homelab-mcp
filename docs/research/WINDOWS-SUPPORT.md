# Windows Support Implementation Plan

> **Status:** Planned
> **Last updated:** 2026-04-05
> **Target:** Full Windows 10/11 support for spacebot-homelab-mcp

---

## Table of Contents

1. [Known Blockers](#known-blockers)
2. [CI/CD Changes](#cicd-changes)
3. [Install Script Changes](#install-script-changes)
4. [Testing Plan](#testing-plan)

---

## Known Blockers

### Blocker 1: `HOME` Environment Variable for Config Path Resolution

| | |
|---|---|
| **File** | `src/config.rs:265-266` |
| **Severity** | Hard blocker (crashes at startup) |

**What it does:**

```rust
let home =
    std::env::var("HOME").map_err(|_| anyhow!("HOME environment variable not set"))?;
PathBuf::from(home)
    .join(".spacebot-homelab")
    .join("config.toml")
```

When no `--config` flag is provided, `Config::load()` reads `HOME` to find the default config at `~/.spacebot-homelab/config.toml`.

**Why it fails on Windows:**

Windows does not set `HOME`. The user's home directory is in `USERPROFILE` (e.g. `C:\Users\Alice`). Some toolchains also set `HOMEDRIVE`+`HOMEPATH`. Without `HOME`, `std::env::var("HOME")` returns `Err` and the server panics with "HOME environment variable not set".

**Fix:**

Introduce a `home_dir()` helper and use it everywhere `HOME` is referenced. Place it at the top of `config.rs`:

```rust
/// Cross-platform home directory resolution.
/// Checks HOME (Unix / Git Bash on Windows), then USERPROFILE (native Windows).
fn home_dir() -> Result<PathBuf> {
    if let Ok(home) = std::env::var("HOME") {
        return Ok(PathBuf::from(home));
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        return Ok(PathBuf::from(profile));
    }
    Err(anyhow!(
        "Could not determine home directory. Set HOME or USERPROFILE."
    ))
}
```

Then update `Config::load()`:

```rust
// Before
let home =
    std::env::var("HOME").map_err(|_| anyhow!("HOME environment variable not set"))?;
PathBuf::from(home)

// After
let home = home_dir()?;
home
    .join(".spacebot-homelab")
    .join("config.toml")
```

Also update the error message on line 276-278:

```rust
// Before
"Configuration file not found at {:?}. Create ~/.spacebot-homelab/config.toml or provide --config <path>",

// After
"Configuration file not found at {:?}. Create it or provide --config <path>.",
```

---

### Blocker 2: `HOME` in `expand_home()` Path Expander

| | |
|---|---|
| **File** | `src/config.rs:375-390` |
| **Severity** | Hard blocker (crashes when config contains `~` paths) |

**What it does:**

```rust
fn expand_home(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    if path_str == "~" {
        return std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| path.to_path_buf());
    }

    if let Some(rest) = path_str.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }

    path.to_path_buf()
}
```

The `~` expansion for SSH key paths, Docker cert paths, and audit log paths all rely on `HOME`.

**Why it fails on Windows:**

Same as Blocker 1 -- `HOME` is not set on native Windows. Additionally, `~` is a valid filename character on Windows, so tilde expansion should only happen with forward-slash notation (`~/...`).

**Fix:**

Rewrite `expand_home()` to use the `home_dir()` helper:

```rust
fn expand_home(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    if path_str == "~" {
        return home_dir()
            .unwrap_or_else(|_| path.to_path_buf());
    }

    if let Some(rest) = path_str.strip_prefix("~/") {
        if let Ok(home) = home_dir() {
            return home.join(rest);
        }
    }

    path.to_path_buf()
}
```

---

### Blocker 3: Docker Named Pipe Support Missing

| | |
|---|---|
| **File** | `src/connection.rs:36-119` |
| **Severity** | Hard blocker (cannot connect to Docker on Windows) |

**What it does:**

`DockerClient::new()` handles two URI schemes:
- `unix://` -- Unix domain socket (macOS/Linux default)
- `tcp://` -- TCP with optional TLS

The fallback at line 114-118 rejects anything else:

```rust
} else {
    return Err(anyhow!(
        "Invalid Docker connection string '{}'. Expected unix:// or tcp://",
        host.host
    ));
};
```

**Why it fails on Windows:**

Docker Desktop for Windows exposes its API over a Windows Named Pipe at `npipe:////./pipe/docker_engine`. The `bollard` crate supports this natively via `Docker::connect_with_named_pipe()`, but the current code doesn't handle the `npipe://` scheme.

**Fix:**

Add a `DockerTransport::NamedPipe` variant and an `npipe://` branch:

```rust
#[derive(Debug, Clone)]
pub enum DockerTransport {
    UnixSocket { path: PathBuf },
    Tcp { host: String, tls: bool },
    NamedPipe { path: String },
}
```

Add the handler before the `else` branch in `DockerClient::new()`:

```rust
} else if host.host.starts_with("npipe://") {
    let pipe_path = &host.host;  // bollard expects the full npipe:// URI
    let client = bollard::Docker::connect_with_named_pipe(
        pipe_path,
        120,
        bollard::API_DEFAULT_VERSION,
    )
    .map_err(|error| {
        anyhow!(
            "Failed to connect to Docker named pipe {}: {}",
            pipe_path,
            error
        )
    })?;

    (
        client,
        DockerTransport::NamedPipe {
            path: pipe_path.to_string(),
        },
    )
}
```

Update `transport_summary()`:

```rust
DockerTransport::NamedPipe { path } => format!("named pipe {}", path),
```

Update the error message:

```rust
"Invalid Docker connection string '{}'. Expected unix://, tcp://, or npipe://",
```

**Note:** `connect_with_named_pipe` only compiles on Windows. Use `#[cfg(windows)]` and `#[cfg(not(windows))]` to conditionally compile the `npipe://` branch. On non-Windows, encountering `npipe://` should return an error saying named pipes are only supported on Windows.

```rust
} else if host.host.starts_with("npipe://") {
    #[cfg(windows)]
    {
        let pipe_path = &host.host;
        let client = bollard::Docker::connect_with_named_pipe(
            pipe_path,
            120,
            bollard::API_DEFAULT_VERSION,
        )
        .map_err(|error| {
            anyhow!("Failed to connect to Docker named pipe {}: {}", pipe_path, error)
        })?;

        (
            client,
            DockerTransport::NamedPipe {
                path: pipe_path.to_string(),
            },
        )
    }
    #[cfg(not(windows))]
    {
        return Err(anyhow!(
            "Named pipe connections (npipe://) are only supported on Windows. Got: '{}'",
            host.host
        ));
    }
}
```

---

### Blocker 4: Hardcoded `/tmp/` in Notification Throttle File

| | |
|---|---|
| **File** | `src/notifications.rs:22` |
| **Severity** | Hard blocker (panics or silent failure on write) |

**What it does:**

```rust
const THROTTLE_FILE: &str = "/tmp/spacebot-homelab-mcp-notify";
```

A temp file is used for cross-process notification deduplication. It's read at line 96, written at line 121, and removed at line 127.

**Why it fails on Windows:**

`/tmp/` does not exist on Windows. Writing to `/tmp/spacebot-homelab-mcp-notify` will fail with a "path not found" error. While the code uses best-effort error handling (most failures are swallowed), `ThrottleGuard::should_notify_failure()` will always return `true`, defeating the deduplication purpose.

**Fix:**

Replace the hardcoded path with a lazily-computed path using `std::env::temp_dir()`:

```rust
use std::sync::LazyLock;

static THROTTLE_FILE: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::temp_dir().join("spacebot-homelab-mcp-notify")
});
const THROTTLE_SECS: u64 = 60;
```

Then update all references from `Path::new(THROTTLE_FILE)` / `THROTTLE_FILE` to `THROTTLE_FILE.as_path()` / `&*THROTTLE_FILE`:

```rust
// Line 96: was Path::new(THROTTLE_FILE)
let path = THROTTLE_FILE.as_path();

// Line 121: was fs::write(THROTTLE_FILE, ...)
let _ = fs::write(THROTTLE_FILE.as_path(), now.to_string());

// Line 127: was fs::remove_file(THROTTLE_FILE)
let _ = fs::remove_file(THROTTLE_FILE.as_path());
```

---

### Blocker 5: Hardcoded `/tmp/` in SSH Download Default Path

| | |
|---|---|
| **File** | `src/tools/ssh.rs:255-263` |
| **Severity** | Hard blocker (download fails with no local_path) |

**What it does:**

```rust
let local_dest = local_path.unwrap_or_else(|| {
    format!(
        "/tmp/homelab-download-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
});
```

When the caller omits `local_path`, the file is downloaded to `/tmp/homelab-download-<timestamp>`.

**Why it fails on Windows:**

`/tmp/` does not exist on Windows. The `fs::write()` at line 299 will fail with "path not found".

**Fix:**

Use `std::env::temp_dir()`:

```rust
let local_dest = local_path.unwrap_or_else(|| {
    let temp = std::env::temp_dir();
    let filename = format!(
        "homelab-download-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    temp.join(filename).to_string_lossy().into_owned()
});
```

---

### Blocker 6: Syslog via `logger` Command

| | |
|---|---|
| **File** | `src/audit.rs:70-82` |
| **Severity** | Soft blocker (syslog audit fails, file audit still works) |

**What it does:**

```rust
async fn write_to_syslog(&self, syslog_config: &crate::config::SyslogConfig, entry: &str) {
    let priority = format!("{}.info", syslog_config.facility);
    let result = tokio::process::Command::new("logger")
        .args(["-p", &priority, "-t", &syslog_config.tag, entry.trim()])
        .output()
        .await;

    if let Err(error) = result {
        tracing::warn!("Failed to write to syslog via logger command: {}", error);
    }
}
```

Shells out to the Unix `logger` command to write syslog entries.

**Why it fails on Windows:**

The `logger` command does not exist on Windows. `Command::new("logger")` will fail with "program not found". The warning will fire on every audit event.

**Fix -- Option A (recommended): Conditional compilation with Windows Event Log stub:**

```rust
async fn write_to_syslog(&self, syslog_config: &crate::config::SyslogConfig, entry: &str) {
    #[cfg(unix)]
    {
        let priority = format!("{}.info", syslog_config.facility);
        let result = tokio::process::Command::new("logger")
            .args(["-p", &priority, "-t", &syslog_config.tag, entry.trim()])
            .output()
            .await;

        if let Err(error) = result {
            tracing::warn!("Failed to write to syslog via logger command: {}", error);
        }
    }

    #[cfg(windows)]
    {
        // Windows has no syslog equivalent. Log a one-time warning.
        // Future enhancement: write to Windows Event Log via `eventlog` crate.
        use std::sync::Once;
        static WARN_ONCE: Once = Once::new();
        WARN_ONCE.call_once(|| {
            tracing::warn!(
                "Syslog audit logging is not supported on Windows. \
                 Configure audit.file instead. (tag={}, facility={})",
                syslog_config.tag,
                syslog_config.facility
            );
        });
    }
}
```

**Fix -- Option B (future): Windows Event Log integration:**

Add the `winlog` or `eventlog` crate behind a `#[cfg(windows)]` dependency and write to the Windows Application event log. This is a larger effort and can be deferred.

---

### Blocker 7: Missing Windows Notification Sounds

| | |
|---|---|
| **File** | `src/notifications.rs:54-77` |
| **Severity** | Soft blocker (notifications work but are silent) |

**What it does:**

```rust
if n.sound {
    #[cfg(target_os = "macos")]
    {
        let sound_name = match n.severity {
            Severity::Success => "Glass",
            Severity::Error => "Basso",
        };
        notif.sound_name(sound_name);
    }

    #[cfg(target_os = "linux")]
    {
        let sound_name = match n.severity {
            Severity::Success => "message-new-instant",
            Severity::Error => "dialog-warning",
        };
        notif.sound_name(sound_name);
        if matches!(n.severity, Severity::Error) {
            notif.urgency(notify_rust::Urgency::Critical);
        }
    }
}
```

Platform-specific sound names for macOS and Linux, but nothing for Windows.

**Why it fails on Windows:**

On Windows, no `#[cfg(target_os = "windows")]` block exists. The `notify-rust` crate supports Windows toast notifications, but they will be silent. The `notify-rust` crate on Windows uses `winrt-notification` under the hood and supports setting sounds via the `sound_name` method using Windows toast audio schema names.

**Fix:**

Add a Windows block:

```rust
#[cfg(target_os = "windows")]
{
    // Windows toast notifications support these sound names.
    // See: https://learn.microsoft.com/en-us/uwp/schemas/tiles/toastschema/element-audio
    let sound_name = match n.severity {
        Severity::Success => "ms-winsoundevent:Notification.Default",
        Severity::Error => "ms-winsoundevent:Notification.Looping.Alarm",
    };
    notif.sound_name(sound_name);
}
```

---

### Blocker 8: SSH `known_hosts` Resolution via `HOME`

| | |
|---|---|
| **File** | `src/connection.rs:201` |
| **Severity** | Hard blocker (SSH connections fail) |

**What it does:**

```rust
match russh::keys::check_known_hosts(&self.host, self.port, server_public_key) {
```

The `russh::keys::check_known_hosts()` function internally looks for `~/.ssh/known_hosts` by reading the `HOME` environment variable (via `dirs::home_dir()` or a direct `HOME` lookup depending on the russh version).

**Why it fails on Windows:**

If `HOME` is not set (typical on native Windows), `russh` may fail to locate `known_hosts`. The error path at line 237-246 will catch this:

```
SSH host key verification failed for host:port: <io error or known_hosts not found>
```

The user-facing error message on line 215 also references a Unix-style path:

```
ssh-keyscan -H {} >> ~/.ssh/known_hosts
```

**Fix:**

1. **Ensure `HOME` is set at process startup.** In `main.rs`, before any SSH operations, detect and set `HOME` if absent:

```rust
// In main(), before Config::load():
#[cfg(windows)]
{
    if std::env::var("HOME").is_err() {
        if let Ok(profile) = std::env::var("USERPROFILE") {
            // Many Rust SSH libraries look for HOME to find ~/.ssh/known_hosts.
            // Set it from USERPROFILE so they work on native Windows.
            unsafe { std::env::set_var("HOME", &profile); }
        }
    }
}
```

2. **Update the error message** on line 214-216 to be cross-platform:

```rust
// Before
"ssh-keyscan -H {} >> ~/.ssh/known_hosts"

// After (platform-aware guidance)
#[cfg(windows)]
let hint = format!(
    "ssh-keyscan -H {} >> %USERPROFILE%\\.ssh\\known_hosts",
    self.host
);
#[cfg(not(windows))]
let hint = format!(
    "ssh-keyscan -H {} >> ~/.ssh/known_hosts",
    self.host
);
```

---

### Blocker 9: Config File Permission Checking (No-Op on Windows)

| | |
|---|---|
| **File** | `src/config.rs:462-466` |
| **Severity** | Soft blocker (security gap, not a crash) |

**What it does:**

```rust
#[cfg(not(unix))]
fn check_config_permissions(_path: &Path) -> Result<()> {
    // Permission checks only supported on Unix
    Ok(())
}
```

On non-Unix platforms (including Windows), config file permissions are not validated at all. The Unix version (lines 427-460) rejects world-readable files.

**Why it's a problem on Windows:**

Config files may contain sensitive paths and credential references. Without ACL checking, a world-readable config on Windows goes undetected. This is a **security gap** rather than a crash.

**Fix -- Phase 1 (log a warning):**

```rust
#[cfg(not(unix))]
fn check_config_permissions(path: &Path) -> Result<()> {
    tracing::info!(
        "Config permission checks are not enforced on this platform. \
         Ensure {:?} is only readable by your user account.",
        path
    );
    Ok(())
}
```

**Fix -- Phase 2 (Windows ACL checking, optional/future):**

Use the `windows-acl` crate or raw Win32 `GetSecurityInfo` / `GetEffectiveRightsFromAcl` APIs to verify:
- The file owner is the current user
- The DACL does not grant `FILE_GENERIC_READ` to `Everyone` or `Users`

This is non-trivial and can be deferred to a follow-up. A warning in Phase 1 is sufficient for initial Windows support.

---

### Blocker 10: `wait_for_shutdown_signal()` -- SIGTERM handling

| | |
|---|---|
| **File** | `src/main.rs:164-179` |
| **Severity** | No blocker (already handled correctly) |

**What it does:**

```rust
async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = signal::ctrl_c() => {},
            _ = sigterm.recv() => {},
        }
    }

    #[cfg(not(unix))]
    {
        let _ = signal::ctrl_c().await;
    }
}
```

**Assessment:** This is **already correctly handled**. The `#[cfg(not(unix))]` block falls back to `ctrl_c()` only, which is the appropriate behavior on Windows. SIGTERM does not exist on Windows. Windows services receive `CTRL_CLOSE_EVENT`, `CTRL_SHUTDOWN_EVENT`, etc., which `ctrl_c()` in tokio covers for console apps.

No changes needed.

---

### Blocker 11: `bollard` SSL Feature on Windows

| | |
|---|---|
| **File** | `Cargo.toml:26` |
| **Severity** | Potential build blocker |

**What it does:**

```toml
bollard = { version = "0.17", features = ["ssl"] }
```

The `ssl` feature links against OpenSSL for Docker TLS connections.

**Why it may fail on Windows:**

Building with the `ssl` feature requires OpenSSL headers and libraries at compile time. On Windows CI runners, OpenSSL is not always pre-installed. `bollard` can also use `rustls` as a TLS backend, which is pure Rust and requires no system dependencies.

**Fix:**

Check if `bollard` 0.17 supports a `rustls` feature flag. If so, consider making the TLS backend conditional:

```toml
# Option A: Use rustls everywhere (simpler, no system deps)
bollard = { version = "0.17", features = ["rustls"] }

# Option B: Platform-conditional (if API differs)
[target.'cfg(windows)'.dependencies]
bollard = { version = "0.17", features = ["rustls"] }

[target.'cfg(unix)'.dependencies]
bollard = { version = "0.17", features = ["ssl"] }
```

If neither is available, the CI workflow must install OpenSSL on Windows (see CI section below). Alternatively, if TLS Docker connections are not needed on Windows, the `ssl` feature can be made optional.

---

## Summary Table

| # | File | Lines | Description | Severity | Fix Complexity |
|---|------|-------|-------------|----------|----------------|
| 1 | `src/config.rs` | 265-266 | `HOME` env var in `Config::load()` | Hard | Low |
| 2 | `src/config.rs` | 375-390 | `HOME` env var in `expand_home()` | Hard | Low |
| 3 | `src/connection.rs` | 36-119 | No `npipe://` Docker support | Hard | Medium |
| 4 | `src/notifications.rs` | 22 | Hardcoded `/tmp/` throttle file | Hard | Low |
| 5 | `src/tools/ssh.rs` | 255-263 | Hardcoded `/tmp/` download path | Hard | Low |
| 6 | `src/audit.rs` | 70-82 | `logger` command for syslog | Soft | Low |
| 7 | `src/notifications.rs` | 54-77 | No Windows notification sounds | Soft | Low |
| 8 | `src/connection.rs` | 201 | SSH `known_hosts` needs `HOME` | Hard | Low |
| 9 | `src/config.rs` | 462-466 | No-op permission check on Windows | Soft | Low (Phase 1) |
| 10 | `src/main.rs` | 164-179 | SIGTERM handling | None | Already done |
| 11 | `Cargo.toml` | 26 | `bollard` SSL build on Windows | Potential | Medium |

---

## CI/CD Changes

### Current State

The release workflow (`.github/workflows/release.yml`) builds for four targets:
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu` (via `cross`)
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`

The CI workflow (`.github/workflows/ci.yml`) runs tests on `ubuntu-latest` and `macos-latest` only.

### Changes to `.github/workflows/release.yml`

Add a Windows build target to the matrix:

```yaml
matrix:
  include:
    # ... existing targets ...
    - target: x86_64-pc-windows-msvc
      os: windows-latest
      archive: zip
```

**Use native `windows-latest` runner** rather than `cross`. `cross` uses Docker containers with Linux sysroots and cannot produce MSVC-linked Windows binaries. The `windows-latest` runner has the MSVC toolchain pre-installed.

Update the **Build** step to handle Windows (no bash `if` statements):

```yaml
- name: Build (Unix)
  if: runner.os != 'Windows'
  run: |
    if [ "${{ matrix.cross }}" = "true" ]; then
      cross build --release --target ${{ matrix.target }}
    else
      cargo build --release --target ${{ matrix.target }}
    fi

- name: Build (Windows)
  if: runner.os == 'Windows'
  run: cargo build --release --target ${{ matrix.target }}
```

Update the **Package** step to produce `.zip` on Windows:

```yaml
- name: Package (Unix)
  if: runner.os != 'Windows'
  id: package-unix
  run: |
    VERSION="${GITHUB_REF_NAME#v}"
    ARCHIVE_NAME="${BINARY_NAME}-${VERSION}-${{ matrix.target }}.${{ matrix.archive }}"
    mkdir -p staging
    cp "target/${{ matrix.target }}/release/${BINARY_NAME}" staging/
    cp README.md LICENSE* example.config.toml staging/ 2>/dev/null || true
    cd staging
    tar czf "../${ARCHIVE_NAME}" .
    cd ..
    echo "archive=${ARCHIVE_NAME}" >> "$GITHUB_OUTPUT"
    echo "version=${VERSION}" >> "$GITHUB_OUTPUT"

- name: Package (Windows)
  if: runner.os == 'Windows'
  id: package-windows
  shell: pwsh
  run: |
    $Version = "${{ github.ref_name }}".TrimStart("v")
    $ArchiveName = "${{ env.BINARY_NAME }}-${Version}-${{ matrix.target }}.${{ matrix.archive }}"
    New-Item -ItemType Directory -Force staging
    Copy-Item "target/${{ matrix.target }}/release/${{ env.BINARY_NAME }}.exe" staging/
    Copy-Item README.md, example.config.toml staging/ -ErrorAction SilentlyContinue
    Compress-Archive -Path staging/* -DestinationPath $ArchiveName
    echo "archive=${ArchiveName}" >> $env:GITHUB_OUTPUT
    echo "version=${Version}" >> $env:GITHUB_OUTPUT
```

Update the **Release** job to collect both `.tar.gz` and `.zip`:

```yaml
- name: Collect archives
  run: |
    mkdir -p release
    find artifacts -type f \( -name '*.tar.gz' -o -name '*.zip' \) -exec mv {} release/ \;
    ls -la release/

- name: Generate checksums
  working-directory: release
  run: |
    sha256sum *.tar.gz *.zip > checksums-sha256.txt
    cat checksums-sha256.txt

- name: Create GitHub Release
  uses: softprops/action-gh-release@v2
  with:
    generate_release_notes: true
    files: |
      release/*.tar.gz
      release/*.zip
      release/checksums-sha256.txt
```

**OpenSSL on Windows:** If the `bollard` `ssl` feature is kept (rather than switching to `rustls`), add a step before the build:

```yaml
- name: Install OpenSSL (Windows)
  if: runner.os == 'Windows'
  run: |
    choco install openssl -y
    echo "OPENSSL_DIR=C:\Program Files\OpenSSL-Win64" >> $env:GITHUB_ENV
```

### Changes to `.github/workflows/ci.yml`

Add `windows-latest` to the test matrix:

```yaml
test:
  name: Test
  runs-on: ${{ matrix.os }}
  strategy:
    matrix:
      os: [ubuntu-latest, macos-latest, windows-latest]
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
    - run: cargo test --lib --all-features
```

Also add a Windows check/clippy job or extend the existing ones:

```yaml
check-windows:
  name: Check (Windows)
  runs-on: windows-latest
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
    - run: cargo check --all-features
    - run: cargo clippy --all-features -- -D warnings
```

---

## Install Script Changes

### Current State

`install.sh` is a bash script that detects platform via `uname`, downloads the correct archive from GitHub Releases, extracts it, and copies the binary to `/usr/local/bin`.

It only supports Linux and macOS (line 20-24):

```bash
case "$os" in
  Linux)  os="unknown-linux-gnu" ;;
  Darwin) os="apple-darwin" ;;
  *)      error "Unsupported OS: $os" ;;
esac
```

### Recommendation

Create a **PowerShell install script** (`install.ps1`) for Windows. Do **not** try to make `install.sh` work on Windows -- bash is not guaranteed on Windows, and the Unix conventions (paths, permissions, sudo) don't translate.

### `install.ps1`

```powershell
#Requires -Version 5.1
# Install script for spacebot-homelab-mcp on Windows
# Usage: irm https://raw.githubusercontent.com/Joshf225/spacebot-homelab-mcp/main/install.ps1 | iex

$ErrorActionPreference = "Stop"

$Repo = "Joshf225/spacebot-homelab-mcp"
$Binary = "spacebot-homelab-mcp"
$InstallDir = if ($env:INSTALL_DIR) { $env:INSTALL_DIR } else { "$env:LOCALAPPDATA\Programs\$Binary" }

function Get-LatestVersion {
    $url = "https://api.github.com/repos/$Repo/releases/latest"
    $release = Invoke-RestMethod -Uri $url -UseBasicParsing
    return $release.tag_name.TrimStart("v")
}

function Get-Platform {
    $arch = if ([Environment]::Is64BitOperatingSystem) { "x86_64" } else {
        throw "32-bit Windows is not supported."
    }
    # ARM64 detection
    if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") {
        throw "ARM64 Windows builds are not yet available."
    }
    return "$arch-pc-windows-msvc"
}

$Version = if ($env:VERSION) { $env:VERSION } else {
    Write-Host "==> Fetching latest release..." -ForegroundColor Cyan
    Get-LatestVersion
}

if (-not $Version) {
    throw "Could not determine latest version. Set `$env:VERSION = 'x.y.z'` manually."
}

$Platform = Get-Platform
Write-Host "==> Installing $Binary v$Version for $Platform" -ForegroundColor Cyan

$Archive = "$Binary-$Version-$Platform.zip"
$Url = "https://github.com/$Repo/releases/download/v$Version/$Archive"
$TempDir = Join-Path $env:TEMP "spacebot-install-$(Get-Random)"

try {
    New-Item -ItemType Directory -Force $TempDir | Out-Null
    $ArchivePath = Join-Path $TempDir $Archive

    Write-Host "==> Downloading $Url..." -ForegroundColor Cyan
    Invoke-WebRequest -Uri $Url -OutFile $ArchivePath -UseBasicParsing

    Write-Host "==> Extracting..." -ForegroundColor Cyan
    Expand-Archive -Path $ArchivePath -DestinationPath $TempDir -Force

    $BinaryPath = Join-Path $TempDir "$Binary.exe"
    if (-not (Test-Path $BinaryPath)) {
        throw "Binary not found in archive"
    }

    # Create install directory
    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Force $InstallDir | Out-Null
    }

    Copy-Item $BinaryPath (Join-Path $InstallDir "$Binary.exe") -Force
    Write-Host "==> Installed $Binary to $InstallDir\$Binary.exe" -ForegroundColor Cyan

    # Add to PATH if not already there
    $UserPath = [Environment]::GetEnvironmentVariable("PATH", "User")
    if ($UserPath -notlike "*$InstallDir*") {
        [Environment]::SetEnvironmentVariable("PATH", "$UserPath;$InstallDir", "User")
        Write-Host "==> Added $InstallDir to user PATH (restart terminal to take effect)" -ForegroundColor Cyan
    }

    Write-Host ""
    Write-Host "==> Next steps:" -ForegroundColor Cyan
    Write-Host "  1. Create config directory: mkdir ~\.spacebot-homelab"
    Write-Host "  2. Copy example config:     copy example.config.toml ~\.spacebot-homelab\config.toml"
    Write-Host "  3. Edit config with your Docker/SSH hosts"
    Write-Host "  4. Validate: $Binary doctor --config ~\.spacebot-homelab\config.toml"
}
finally {
    Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
}
```

### Update `install.sh`

Optionally add a check at the top of `install.sh` to redirect Windows/MSYS users:

```bash
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    echo "Detected Windows environment. Use install.ps1 instead:"
    echo "  irm https://raw.githubusercontent.com/Joshf225/spacebot-homelab-mcp/main/install.ps1 | iex"
    exit 1
    ;;
esac
```

---

## Testing Plan

### 1. CI-Based Testing (Automated)

**Unit tests on `windows-latest`:**
Add `windows-latest` to the CI test matrix (see CI/CD section). This catches compile errors and logic failures in unit tests.

```yaml
os: [ubuntu-latest, macos-latest, windows-latest]
```

All existing unit tests should pass unchanged. Tests that use `tempfile` (like the permission tests in `config.rs`) already skip via `#[cfg(unix)]`.

**Build verification:**
The release workflow will attempt to build `x86_64-pc-windows-msvc`, catching any link errors (e.g., OpenSSL, missing system libraries).

### 2. Integration Tests with Docker (Automated, CI)

On `windows-latest` runners, Docker is available (Docker Desktop is pre-installed on GitHub Actions Windows runners). Add a CI job:

```yaml
integration-windows:
  name: Integration (Windows + Docker)
  runs-on: windows-latest
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
    - name: Start Docker
      run: |
        # Docker Desktop should be available; verify
        docker info
    - name: Run integration tests
      run: cargo test --test docker_integration
      env:
        SPACEBOT_HOMELAB_NO_NOTIFY: "1"
```

**Note:** GitHub Actions `windows-latest` runners have Docker available with `npipe:////./pipe/docker_engine` as the endpoint. This validates the named pipe code path.

### 3. Manual Testing Checklist

For the initial Windows release, manually verify on a Windows 10/11 machine:

- [ ] **Binary starts:** `spacebot-homelab-mcp.exe server --config config.toml`
- [ ] **Config loading:** Config at `%USERPROFILE%\.spacebot-homelab\config.toml` is found automatically (without `--config`)
- [ ] **Tilde expansion:** `private_key_path = "~/.ssh/id_ed25519"` resolves correctly
- [ ] **Docker named pipe:** Connect to local Docker Desktop via `npipe:////./pipe/docker_engine`
- [ ] **Docker TCP:** Connect to a remote Docker host via `tcp://`
- [ ] **SSH connection:** Connect to a remote host, verify `known_hosts` lookup works
- [ ] **SSH download:** Default download path goes to `%TEMP%\homelab-download-*`
- [ ] **Notification:** Desktop toast appears on connect (with sound)
- [ ] **Notification throttle:** Rapid restarts don't spam notifications
- [ ] **Syslog warning:** With `audit.syslog` configured, a warning appears once (not per-event)
- [ ] **Doctor command:** `spacebot-homelab-mcp.exe doctor --config config.toml` runs cleanly
- [ ] **Ctrl+C shutdown:** Server shuts down gracefully on Ctrl+C
- [ ] **Metrics endpoint:** (if enabled) `http://127.0.0.1:9090/metrics` returns Prometheus data

### 4. Test Matrix Summary

| Test Type | Linux | macOS | Windows |
|-----------|-------|-------|---------|
| Unit tests (`cargo test --lib`) | CI | CI | CI (new) |
| Clippy + fmt | CI | -- | CI (new) |
| Build release binary | CI | CI | CI (new) |
| Docker integration | Manual | Manual | CI + Manual |
| SSH integration | Manual | Manual | Manual |
| Install script | `install.sh` | `install.sh` | `install.ps1` |

---

## Implementation Order

Recommended order of implementation, prioritizing hard blockers:

1. **Add `home_dir()` helper** and fix Blockers 1, 2, 8 (all `HOME`-related). Single PR.
2. **Fix hardcoded `/tmp/` paths** -- Blockers 4, 5. Single PR.
3. **Add `npipe://` Docker support** -- Blocker 3. May require Cargo.toml changes (Blocker 11). Single PR.
4. **Fix syslog and notification sounds** -- Blockers 6, 7. Single PR.
5. **Update config permission check** -- Blocker 9. Single PR (Phase 1 warning only).
6. **CI/CD: Add Windows to CI and release workflows**. Single PR.
7. **Create `install.ps1`**. Single PR.
8. **Manual QA pass** on a Windows machine.

Total estimated effort: **2-3 days** for an experienced Rust developer.
