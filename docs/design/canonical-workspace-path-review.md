# Expert Panel Review: Canonical Workspace Path Projection

**Reviewed document**: [canonical-workspace-path.md](file:///Users/antigravity/rust_source/velo-rift/docs/design/canonical-workspace-path.md)

---

## Panel Members

| Expert | Domain | Focus |
|--------|--------|-------|
| **A — OS / Systems** | macOS DYLD, syscall interposition, VFS | Shim feasibility, performance, correctness |
| **B — Rust / Cargo Internals** | Cargo fingerprint, rustc, incremental compilation | Whether fingerprints actually become path-portable |
| **C — Build Systems & Caching** | sccache, Bazel, Nix, ccache | Architecture comparison, known pitfalls |
| **D — Security & Reliability** | Path traversal, TOCTOU, race conditions | Attack surface, failure modes |

---

## Expert A: OS / Systems

### ✅ Strengths

1. **Per-process isolation via DYLD_INSERT_LIBRARIES is sound.** Each process tree inherits its own `VRIFT_PHYSICAL_ROOT`, no global state conflict. This is proven by the existing inception layer which already uses this exact mechanism.

2. **Existing infrastructure covers ~80% of needed syscalls.** The current `variadic_inception.c` already intercepts `open/openat`, `stat/lstat/fstatat`, `access`, `readlink`, `rename/renameat`, `fstat`, `creat`, `getattrlist/setattrlist`. The path filtering logic (`path_needs_inception`) already checks against `PROJECT_ROOT`.

### ⚠️ Concerns

3. **`getcwd()` interception is the hardest part and is underspecified.**
   - The document says "intercept `getcwd()`", but on macOS, `getcwd()` is implemented via the `__getcwd` syscall (not `fcntl`). This syscall writes directly to a user buffer — the shim must intercept the raw syscall return and rewrite the buffer contents.
   - More critically: **the kernel tracks the real cwd**. If the process does `chdir("/workspace/src")`, the shim translates to `chdir("$PHYSICAL_ROOT/src")`, and the kernel records `$PHYSICAL_ROOT/src`. But then `getcwd()` returns the kernel's real path. The shim must detect prefix `$PHYSICAL_ROOT` and replace it with `/workspace`.
   - **Risk**: If the physical root contains symlinks, `getcwd()` may return the symlink-resolved path, which won't match `$PHYSICAL_ROOT`. The shim must `realpath($PHYSICAL_ROOT)` at init time and match against both.

4. **`realpath()` is a libc function, not a syscall.**
   - It's implemented in userspace using `stat()` + `readlink()` + `lstat()`. Since those are already intercepted, `realpath()` might "just work" — but it depends on the libc implementation. On macOS, `realpath()` may call `getattrlist` or `fcntl(F_GETPATH)` which gives the kernel's canonical path. **`fcntl(F_GETPATH)` MUST be intercepted** — it returns the absolute path for an fd and is used by many Rust standard library functions.

5. **`/proc/self/fd` equivalent on macOS.**
   - macOS doesn't have `/proc`, but `fcntl(fd, F_GETPATH, buf)` serves the same purpose. If any tool calls this (and many do), it will return the physical path. **This is a gap in the current interception list.**

6. **Performance: path string comparison on every syscall.**
   - The `translate_inbound` function does `strncmp(path, "/workspace/", 11)` on every path syscall. This is negligible individually but is called thousands of times during a cargo build. Recommendation: since `/workspace` starts with `/w`, a single character check `path[1] == 'w'` as fast-reject would help, though the current `path_needs_inception()` already does similar prefix checks efficiently.

### 🔧 Recommendations

- **Add `fcntl(F_GETPATH)` to the interception list** — this is a critical gap
- **Resolve `PHYSICAL_ROOT` via `realpath()` at init** to handle symlinks
- **Document `getcwd()` interception as the highest-risk item** — prototype this first

---

## Expert B: Rust / Cargo Internals

### ✅ Strengths

1. **Correct identification of path-bearing files.** The document correctly identifies the three file types containing absolute paths: `.d` files, `dep-info` binary files, and `build/*/output`. The fingerprint JSON uses hashes, not literal paths — good observation.

2. **`CARGO_MANIFEST_DIR` will naturally be canonical.** Cargo sets this from its own notion of the project root. If Cargo sees `/workspace/Cargo.toml` (because `getcwd()` returns `/workspace`), then `CARGO_MANIFEST_DIR=/workspace/crates/foo`. Correct.

### ⚠️ Concerns

3. **Cargo's `dep-info` format is not pure binary — it's a custom packed format.**
   - The format is: `[1-byte length][path bytes]` repeated, with some paths being relative and some absolute. Relative paths are resolved against the package root. If `getcwd()` returns `/workspace`, relative paths resolve correctly. But **absolute paths in dep-info are written by rustc**, not cargo. They come from:
     - `include!()` macro referencing absolute paths
     - Build script `OUT_DIR` (generated code)
     - `env!("CARGO_MANIFEST_DIR")` at compile time
   - If all of these see `/workspace/...`, the dep-info will contain canonical paths. **This should work correctly.**

4. **`Cargo.lock` contains registry source URLs, not paths — ✅ no issue.**

5. **Incremental compilation directory (`target/debug/incremental/`).**
   - The design doesn't mention this. Incremental compilation caches contain binary data with embedded paths (file paths in `work-products.json` and `.o` object files with debug info). If caching incremental artifacts, those paths must also be canonical.
   - **Recommendation**: Either exclude `target/debug/incremental/` from caching (simpler, small perf cost on restore) or accept that incremental cache is path-sensitive and non-portable.

6. **`--remap-path-prefix` interaction is subtle.**
   - The document proposes `--remap-path-prefix=/workspace=$PHYSICAL_ROOT` for error messages. This is correct for diagnostics. However, `--remap-path-prefix` ALSO affects:
     - `file!()` macro output
     - `panic!()` location strings
     - `#[track_caller]` output
   - If a library compiled under inception uses `file!()`, it will return the remapped (physical) path. If that library is then used in a different directory, `file!()` returns the old physical path embedded at compile time. **This is acceptable** — `file!()` is rarely used for filesystem operations, mostly for logging.

7. **`path` field in fingerprint JSON is a hash of the path, not the path itself.**
   - Cargo computes `path = hash(package_root_relative_path_to_source_file)`. Since `package_root` is `/workspace/crates/foo` under inception, this hash will be consistent. ✅ Correct.

8. **Registry source paths in dep-info.**
   - Third-party crates have source at `~/.cargo/registry/src/index.crates.io-.../cratename-version/src/lib.rs`. This absolute path appears in `.d` files and dep-info. **Phase 4 (cargo-registry canonicalization) is essential for cross-machine sharing**, but can be deferred for single-machine cross-directory sharing.

### 🔧 Recommendations

- **Explicitly exclude `target/debug/incremental/` from cache**, at least initially
- **Document `file!()` / `panic!()` behavior** — these embed paths at compile time and survive across builds
- **Phase 4 priority**: For single-machine, same-user use cases, registry paths are identical anyway. Phase 4 is only needed for cross-machine.

---

## Expert C: Build Systems & Caching

### ✅ Strengths

1. **Fundamentally better architecture than sccache for this use case.**
   - sccache works at rustc level → can only cache `.rlib`/`.rmeta`, not fingerprints
   - This design works at VFS level → everything (fingerprints, dep-info, build scripts) naturally uses canonical paths
   - No post-hoc rewriting needed — artifacts are born with portable paths

2. **Content-based project ID (Phase 3) is the right approach.** Bazel and Nix both use content-addressable keys. Hashing `Cargo.lock + Cargo.toml + rustc version` makes cache sharing deterministic.

### ⚠️ Concerns

3. **Cache key design needs more thought.**
   - `Cargo.lock + Cargo.toml + rustc version` is necessary but not sufficient. Must also include:
     - **Target triple** (cross-compilation)
     - **Profile** (dev/release/test)
     - **Features** (different feature sets produce different artifacts)
     - **RUSTFLAGS** (affects all compilation)
     - **Environment variables that Cargo passes to rustc** (e.g., `CARGO_CFG_*`)
   - Recommendation: use the same key components that sccache uses, or defer to per-crate fingerprints which already encode all of this.

4. **Stale cache invalidation.**
   - The document doesn't discuss cache eviction or staleness detection. If `Cargo.lock` changes (dependency update), the old cache should be invalidated. Content-based keys handle this naturally, but storage cleanup (GC) is needed.
   - Bazel's approach: LRU eviction with configurable max cache size. Recommending similar.

5. **Comparison to `cargo-cache` and `cargo-binstall`.**
   - `cargo-cache` already caches compiled crates in `~/.cargo`. It doesn't handle fingerprints. This design is complementary, not competing.
   - Nix-based Rust builds (naersk, crane) use content-addressed stores with path canonicalization at build time. The `/workspace` approach mirrors Nix's `/nix/store` — a canonical path that decouples content from physical location. This is a proven pattern.

### 🔧 Recommendations

- **Start with path-based project ID** (current implementation) for Phase 1-2. Switch to content-based in Phase 3.
- **Add cache GC** with max size limit (e.g., 10GB default)
- **Consider Cargo's own `-Z gc` unstable feature** — Cargo is adding built-in GC for global caches

---

## Expert D: Security & Reliability

### ✅ Strengths

1. **No root privileges required.** DYLD-based, userspace only.
2. **Scope is limited.** Only paths under `$PHYSICAL_ROOT` are remapped. System paths (`/usr`, `/lib`) pass through untouched.

### ⚠️ Concerns

3. **`/workspace` path collision.**
   - The document mentions this but the mitigation is weak. "Use `/.vrift/workspace`" — but paths starting with `/.` are unusual and may confuse tools.
   - **Better alternative**: Use `/dev/fd/../vrift-workspace-XXXX` or a path under `/tmp/vrift-XXXX` which is guaranteed unique per session. However, this defeats the determinism needed for caching. 
   - **Best alternative**: Use a fixed path like `/vrift/w` (short, unlikely to conflict, and deterministic across machines). The path length matters because it appears in every fingerprint and dep-info file — shorter saves space.
   - **Critical constraint**: The canonical path length MUST be ≤ physical path length if doing in-place binary rewriting. Since we're intercepting instead of rewriting, this constraint doesn't apply. ✅

4. **TOCTOU between path translation and syscall.**
   - If the physical root directory is a symlink or is renamed while a build is in progress, the translated path could point to the wrong location. This is unlikely in practice but worth documenting as a known limitation.

5. **SIP (System Integrity Protection) on macOS.**
   - `DYLD_INSERT_LIBRARIES` is stripped for SIP-protected binaries (`/usr/bin/*`). If any build script invokes a SIP-protected binary with a `/workspace` path argument (not via open() but as argv), the binary won't have the shim and will fail to find `/workspace`.
   - **Mitigation**: This is already handled by the existing inception layer — SIP-protected binaries are expected to not access VFS paths directly. Build scripts rarely invoke `/usr/bin/*` with project-relative paths.

6. **Environment variable leakage.**
   - `VRIFT_PHYSICAL_ROOT` is visible in `/proc/environ` (or via `ps eww`). On shared machines, this leaks the user's physical project path. Low risk for most environments but worth noting for enterprise/CI contexts.

### 🔧 Recommendations

- **Use `/vrift/w` as canonical root** — short, deterministic, unlikely collision
- **Document SIP limitation** explicitly
- **Sanitize env vars** for subprocesses that shouldn't see inception internals (already needed for existing inception layer)

---

## Consensus Summary

| Area | Verdict | Key Action |
|------|---------|------------|
| Overall approach | **✅ Approved** | Sound architecture, better than alternatives |
| Syscall coverage | **⚠️ Gap** | **Must add `fcntl(F_GETPATH)`** interception |
| `getcwd()` interception | **⚠️ High risk** | Prototype first, handle symlink resolution |
| Fingerprint portability | **✅ Will work** | All path-bearing data flows through intercepted syscalls |
| Incremental compilation | **✅ Safe** | VFS interception means paths are born canonical — no binary rewriting needed |
| Cache key design | **⚠️ Incomplete** | Must include target/profile/features, defer to Phase 3 |
| Canonical path choice | **✅ Decided** | `/vrift_workspace` — readable, no collision risk |
| `--remap-path-prefix` | **✅ Correct** | Acceptable for IDE integration |
| Security | **✅ Low risk** | Minor edge cases, well-mitigated |

### Priority Order for Implementation

1. **`fcntl(F_GETPATH)` + `getcwd()` interception** — prototype and validate
2. **Phase 1** (shim translation) — builds on existing infrastructure
3. **Phase 2** (chdir tracking) — essential for cargo
4. **Phase 3** (cache key) — defer until cross-directory sharing is needed
5. **Phase 4** (registry/rustup) — defer until cross-machine sharing is needed
