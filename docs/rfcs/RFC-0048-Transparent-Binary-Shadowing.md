# RFC-0048: SIP Bypass via PATH Shim (Inception Mode)

## Status: PROPOSED

> **Original Approach (Binary Shadowing)**: ❌ NOT VIABLE  
> **New Approach (PATH Shim / Inception Mode)**: ✅ PROPOSED

---

## 1. Problem Statement

On macOS, **System Integrity Protection (SIP)** prevents `DYLD_INSERT_LIBRARIES` from injecting into binaries in protected directories (`/bin`, `/usr/bin`, etc.). When build systems invoke shell commands like `chmod`, `rm`, `cp`, the Velo Rift shim is bypassed.

### Scope of Impact

| Call Type | Shim Intercepts? | Example |
|-----------|-----------------|---------|
| Direct syscall (C/Rust) | ✅ Yes | `chmod("file", 0755)` |
| Python/Node/Go stdlib | ✅ Yes | `os.chmod()`, `fs.chmod()` |
| Shell command | ❌ No | `$(shell chmod 755 file)` |
| Makefile shell | ❌ No | `chmod 755 $(TARGET)` |

---

## 2. Failed Approach: Binary Shadowing

### 2.1 Approach

Copy SIP-protected binaries to a shadow directory, re-sign with ad-hoc codesign, and redirect execution.

### 2.2 Result: ❌ BLOCKED

> [!CAUTION]
> **macOS Platform Binary Enforcement**: Apple-signed binaries carry `Platform identifier=15`. When copied and re-signed, the macOS kernel kills the process with SIGKILL.

This restriction cannot be bypassed without disabling SIP (requires Recovery Mode) or kernel extensions.

---

## 3. Proposed Solution: PATH Shim (Inception Mode)

### 3.1 Concept: "Inception"

Inspired by the movie *Inception* (盗梦空间), entering VFS mode is like entering a dream layer:

- **`vrift inception`** - Enter the dream (activate VFS environment)
- **`vrift wake`** - Exit the dream (deactivate VFS environment)

### 3.2 Industry Precedent

| Tool | Approach | 
|------|----------|
| **ccache/sccache** | PATH symlinks to wrapper |
| **asdf/mise** | Shim directory with version resolution |
| **direnv** | Shell hook + auto-activation |
| **rustup** | Shim proxies to correct toolchain |

### 3.3 Architecture

```
                    Shell executes "chmod 755 file"
                              ↓
                    Search $PATH for "chmod"
                              ↓
            ┌─────────────────────────────────────┐
            │ $PROJECT/.vrift/bin/chmod (wrapper) │ ← Found first
            └─────────────────────────────────────┘
                              ↓
                    Wrapper checks target path
                              ↓
        ┌─────────────────┬─────────────────────────┐
        │ In VFS project? │                         │
        ↓                 ↓                         ↓
       YES               NO                        
        ↓                 ↓                         
   vrift-chmod       /bin/chmod                  
   (loads shim)      (passthrough)               
```

### 3.4 Commands

#### `vrift inception` - Enter the Dream

```bash
$ cd my-project
$ eval "$(vrift inception)"
🌀 Inception: Entering VFS layer for /Users/you/my-project
   PATH prepended with .vrift/bin/
   DYLD_INSERT_LIBRARIES set
   Ready for build...
```

Output (for `eval`):
```bash
export VRIFT_PROJECT_ROOT="/Users/you/my-project"
export VRIFT_INCEPTION=1
export PATH="/Users/you/my-project/.vrift/bin:$PATH"
export DYLD_INSERT_LIBRARIES="/Users/you/my-project/.vrift/libvrift_shim.dylib"
export DYLD_FORCE_FLAT_NAMESPACE=1
echo "🌀 Inception: Entering VFS layer for $VRIFT_PROJECT_ROOT"
```

#### `vrift wake` - Exit the Dream

```bash
$ vrift wake
💫 Wake: Exiting VFS layer
   Environment restored
```

### 3.5 Shell Hook (Optional Auto-Inception)

For automatic activation when entering a VFS project directory:

```bash
# Add to ~/.bashrc or ~/.zshrc
eval "$(vrift hook bash)"   # or zsh/fish
```

Hook behavior:
- `cd my-project/` → Auto-run `vrift inception` if `.vrift/` exists
- `cd ../` → Auto-run `vrift wake` when leaving project

### 3.6 Wrapper Scripts

`.vrift/bin/chmod`:
```bash
#!/bin/bash
# Inception-aware chmod wrapper

TARGET="${@: -1}"
[[ "$TARGET" != /* ]] && TARGET="$(pwd)/$TARGET"

if [[ "$TARGET" == "$VRIFT_PROJECT_ROOT"* ]]; then
    exec "$VRIFT_PROJECT_ROOT/.vrift/helpers/vrift-chmod" "$@"
else
    exec /bin/chmod "$@"
fi
```

### 3.7 Helper Binaries

Compiled binaries that **will** load the shim (not in SIP-protected paths):

- `.vrift/helpers/vrift-chmod`
- `.vrift/helpers/vrift-chown`
- `.vrift/helpers/vrift-rm`
- `.vrift/helpers/vrift-cp`
- `.vrift/helpers/vrift-mv`
- `.vrift/helpers/vrift-touch`

---

## 4. User Experience

```bash
# One-time setup (optional auto-inception)
echo 'eval "$(vrift hook bash)"' >> ~/.bashrc

# Manual inception
$ cd my-project
$ eval "$(vrift inception)"
🌀 Inception: Entering VFS layer for /Users/you/my-project

# Build now works with full VFS interception
$ make build   # Makefile's chmod calls are now intercepted!

# Exit when done
$ vrift wake
💫 Wake: Exiting VFS layer

# Or just close the terminal
```

---

## 5. Implementation Plan

### Phase 1: CLI Commands
- [ ] Implement `vrift inception` - output shell env setup
- [ ] Implement `vrift wake` - output shell env cleanup
- [ ] Implement `vrift hook <shell>` - output shell hook code

### Phase 2: Wrapper Generation
- [ ] `vrift init` generates `.vrift/bin/` wrappers
- [ ] Generate wrappers for: `chmod`, `chown`, `rm`, `cp`, `mv`, `touch`, `mkdir`, `rmdir`

### Phase 3: Helper Binaries
- [ ] Compile shim-loadable helper binaries
- [ ] Bundle with vrift distribution or generate on `vrift init`

### Phase 4: Documentation
- [ ] Update USAGE.md with Inception Mode guide
- [ ] Add examples for common build systems (Make, CMake, npm scripts)

---

## 6. Comparison with Previous Approach

| Aspect | Binary Shadowing | Inception Mode |
|--------|------------------|----------------|
| macOS Compatibility | ❌ Blocked by kernel | ✅ Works |
| Zero Configuration | ✅ Intended | ⚠️ Requires `vrift inception` |
| Project Isolation | ✅ Yes | ✅ Yes |
| Performance | Good (cached) | Good (instant) |
| Security Changes | None | None |
| User Experience | Transparent | Explicit (like virtualenv) |
