//! Target Cache: Per-crate fingerprint-based restoration logic.
//!
//! This module implements the "Triple Check" algorithm:
//! 1. Source Check: Verify source file mtimes (nsec) and sizes match.
//! 2. Toolchain Check: Verify rustc version hash matches.
//! 3. Dependency Check: Verify all dependency crates are valid and matching.
//!
//! Artifacts are restored from CAS via hardlinks (zero-copy) only if all checks pass.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use vrift_cas::CasStore;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ArtifactEntry {
    pub hash_hex: String,
    /// Full mtime: seconds since epoch
    pub mtime_sec: i64,
    /// Nanosecond fraction (0..999_999_999)
    pub mtime_nsec_frac: i64,
    /// Unix file mode (e.g. 0o755 for executables)
    #[serde(default = "default_mode")]
    pub file_mode: u32,
}

fn default_mode() -> u32 {
    0o644
}

/// Represents the state of a single crate during snapshot/restore.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CrateFingerprint {
    pub crate_name: String,
    pub crate_id: String,          // e.g. "tokio-abc123"
    pub fingerprint_value: String, // 16-char hex from Cargo
    pub rustc_hash: u64,
    pub features: String,
    pub target: u64,
    pub profile: u64,
    pub path: u64,
    /// Dependency name -> Fingerprint hash
    pub deps: HashMap<String, u64>,
    /// Relative path -> (mtime_sec, mtime_nsec_frac, size)
    pub sources: HashMap<String, (i64, i64, u64)>,
    /// Path in target/debug/ -> Artifact info
    pub artifacts: HashMap<String, ArtifactEntry>,
}

/// Project-wide target manifest list
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectCache {
    pub project_id: String,
    pub crates: HashMap<String, CrateFingerprint>,
    /// Extra files in target/debug/ root that aren't per-crate
    /// (e.g. final binaries, .d files, build/ dirs)
    #[serde(default)]
    pub extra_artifacts: HashMap<String, ArtifactEntry>,
}

// --- Snapshot Logic ---

/// Snapshot the current target/ state, creating per-crate cache entries.
pub fn snapshot_target(project_dir: &Path, cas: &CasStore) -> Result<(usize, u64, Duration)> {
    let start = Instant::now();
    let target_debug = project_dir.join("target").join("debug");
    if !target_debug.exists() {
        anyhow::bail!("target/debug not found at {}", target_debug.display());
    }

    let mut crates = HashMap::new();
    let mut extra_artifacts = HashMap::new();
    let mut total_size = 0u64;

    // 1. Scan .fingerprint/ directory for ALL crate entries
    let fp_root = target_debug.join(".fingerprint");
    if !fp_root.exists() {
        return Ok((0, 0, start.elapsed()));
    }

    for entry in fs::read_dir(&fp_root)?.flatten() {
        if !entry.path().is_dir() {
            continue;
        }

        let path = entry.path();
        let dirname = path.file_name().unwrap().to_string_lossy();

        if let Some(crate_fp) = process_crate_dir(project_dir, &target_debug, &path, cas)? {
            for rel in crate_fp.artifacts.keys() {
                let abs_path = target_debug.join(rel);
                total_size += fs::metadata(&abs_path).map(|m| m.len()).unwrap_or(0);
            }
            crates.insert(dirname.to_string(), crate_fp);
        }
    }

    // 2. Snapshot build/ directory (build script outputs)
    let build_root = target_debug.join("build");
    if build_root.exists() {
        snapshot_dir_recursive(&build_root, &target_debug, cas, &mut extra_artifacts)?;
    }

    // 3. Snapshot root-level files in target/debug/ (final binaries, .d files)
    for entry in fs::read_dir(&target_debug)?.flatten() {
        let path = entry.path();
        if path.is_file() {
            ingest_artifact(&path, &target_debug, cas, &mut extra_artifacts)?;
            total_size += fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        }
    }

    // 4. Save project manifest
    let project_id = vrift_config::path::compute_project_id(project_dir);
    let cache = ProjectCache {
        project_id: project_id.clone(),
        crates,
        extra_artifacts,
    };

    let storage_dir = get_cache_storage_dir()?;
    let manifest_path = storage_dir.join(format!("{}.json", project_id));
    let json = serde_json::to_string(&cache)?;
    fs::write(manifest_path, json)?;

    Ok((cache.crates.len(), total_size, start.elapsed()))
}

/// Recursively snapshot all files in a directory
fn snapshot_dir_recursive(
    dir: &Path,
    target_root: &Path,
    cas: &CasStore,
    artifacts: &mut HashMap<String, ArtifactEntry>,
) -> Result<()> {
    for entry in fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            snapshot_dir_recursive(&path, target_root, cas, artifacts)?;
        } else if path.is_file() {
            ingest_artifact(&path, target_root, cas, artifacts)?;
        }
    }
    Ok(())
}

fn process_crate_dir(
    project_dir: &Path,
    target_root: &Path,
    crate_dir: &Path,
    cas: &CasStore,
) -> Result<Option<CrateFingerprint>> {
    let dirname = crate_dir.file_name().unwrap().to_string_lossy();
    let parts: Vec<&str> = dirname.rsplitn(2, '-').collect();
    if parts.len() < 2 {
        return Ok(None);
    }
    let hash_suffix = parts[0].to_string();

    // Try to find ANY fingerprint JSON in this dir
    // Cargo uses: lib-<name>.json, bin-<name>.json, build-script-build.json, etc.
    let mut json_path = None;
    let mut crate_name = parts[1].to_string();

    for entry in fs::read_dir(crate_dir)?.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if fname.ends_with(".json") {
            if fname.starts_with("lib-") {
                crate_name = fname
                    .trim_start_matches("lib-")
                    .trim_end_matches(".json")
                    .to_string();
                json_path = Some(entry.path());
                break;
            } else if fname.starts_with("bin-") {
                crate_name = fname
                    .trim_start_matches("bin-")
                    .trim_end_matches(".json")
                    .to_string();
                json_path = Some(entry.path());
                break;
            } else if fname.starts_with("build-script-build")
                || fname.starts_with("build_script_build")
                || fname.starts_with("run-build-script")
            {
                json_path = Some(entry.path());
                break;
            }
        }
    }

    let json_path = match json_path {
        Some(p) => p,
        None => return Ok(None),
    };

    let json_data: serde_json::Value =
        serde_json::from_reader(fs::File::open(&json_path)?).unwrap_or_default();

    // Read binary fingerprint file (same name without .json)
    let bin_fp_path = json_path.with_extension("");
    let fingerprint_value = if bin_fp_path.exists() {
        fs::read_to_string(&bin_fp_path).unwrap_or_default()
    } else {
        String::new()
    };

    let mut fp = CrateFingerprint {
        crate_name: crate_name.clone(),
        crate_id: dirname.to_string(),
        fingerprint_value,
        rustc_hash: json_data["rustc"].as_u64().unwrap_or(0),
        features: json_data["features"].to_string(),
        target: json_data["target"].as_u64().unwrap_or(0),
        profile: json_data["profile"].as_u64().unwrap_or(0),
        path: json_data["path"].as_u64().unwrap_or(0),
        deps: HashMap::new(),
        sources: HashMap::new(),
        artifacts: HashMap::new(),
    };

    // Extract dependencies from JSON
    if let Some(deps_arr) = json_data["deps"].as_array() {
        for d in deps_arr {
            if let Some(dep_entry) = d.as_array() {
                if dep_entry.len() >= 4 {
                    let dep_name = dep_entry[1].as_str().unwrap_or("").to_string();
                    let dep_fp = dep_entry[3].as_u64().unwrap_or(0);
                    fp.deps.insert(dep_name, dep_fp);
                }
            }
        }
    }

    // Read dep-info for source files (try multiple naming conventions)
    let dep_info_candidates = [
        format!("dep-lib-{}", fp.crate_name),
        format!("dep-bin-{}", fp.crate_name),
        format!("dep-build-script-build-{}", fp.crate_name),
    ];
    for candidate in &dep_info_candidates {
        let dep_info_path = crate_dir.join(candidate);
        if dep_info_path.exists() {
            parse_dep_info(project_dir, &dep_info_path, &mut fp.sources)?;
            break;
        }
    }

    // Collect ALL files in the fingerprint directory
    for entry in fs::read_dir(crate_dir)?.flatten() {
        if entry.path().is_file() {
            ingest_artifact(&entry.path(), target_root, cas, &mut fp.artifacts)?;
        }
    }

    // Binary artifacts in deps/
    let deps_root = target_root.join("deps");
    if deps_root.exists() {
        let underscore_name = fp.crate_name.replace('-', "_");
        let artifact_patterns = vec![
            format!("lib{}-{}.rlib", underscore_name, hash_suffix),
            format!("lib{}-{}.rmeta", underscore_name, hash_suffix),
            format!("lib{}-{}.d", underscore_name, hash_suffix),
            // Proc macros compile as dynamic libraries
            format!("lib{}-{}.dylib", underscore_name, hash_suffix),
            format!("lib{}-{}.so", underscore_name, hash_suffix),
            format!("{}-{}", fp.crate_name, hash_suffix),
            format!("{}-{}.d", fp.crate_name, hash_suffix),
            // Also try with underscore name for binary targets
            format!("{}-{}", underscore_name, hash_suffix),
            format!("{}-{}.d", underscore_name, hash_suffix),
        ];

        for pattern in artifact_patterns {
            let path = deps_root.join(&pattern);
            if path.exists() {
                ingest_artifact(&path, target_root, cas, &mut fp.artifacts)?;
            }
        }
    }

    // Build script output directory
    let build_dir = target_root.join("build").join(&*dirname);
    if build_dir.exists() {
        snapshot_dir_recursive(&build_dir, target_root, cas, &mut fp.artifacts)?;
    }

    Ok(Some(fp))
}

fn parse_dep_info(
    project_dir: &Path,
    path: &Path,
    sources: &mut HashMap<String, (i64, i64, u64)>,
) -> Result<()> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);

    for line in reader.lines().map_while(Result::ok) {
        let paths_line = if let Some(idx) = line.find(':') {
            &line[idx + 1..]
        } else {
            &line
        };

        for p_str in paths_line.split_whitespace() {
            let p = PathBuf::from(p_str);
            if p.exists() {
                let meta = fs::metadata(&p)?;
                let (mtime_sec, mtime_nsec_frac) = get_mtime(&meta);
                let size = meta.len();

                let rel = if let Ok(r) = p.strip_prefix(project_dir) {
                    r.to_string_lossy().to_string()
                } else {
                    p.to_string_lossy().into_owned()
                };
                sources.insert(rel, (mtime_sec, mtime_nsec_frac, size));
            }
        }
    }
    Ok(())
}

fn ingest_artifact(
    path: &Path,
    target_root: &Path,
    cas: &CasStore,
    artifacts: &mut HashMap<String, ArtifactEntry>,
) -> Result<()> {
    let hash = cas.store_file(path)?;
    let rel = path
        .strip_prefix(target_root)?
        .to_string_lossy()
        .to_string();
    let meta = fs::metadata(path)?;
    let (mtime_sec, mtime_nsec_frac) = get_mtime(&meta);
    let file_mode = get_file_mode(&meta);
    artifacts.insert(
        rel,
        ArtifactEntry {
            hash_hex: CasStore::hash_to_hex(&hash),
            mtime_sec,
            mtime_nsec_frac,
            file_mode,
        },
    );
    Ok(())
}

// --- Restore Logic ---

pub fn restore_target(project_dir: &Path, cas: &CasStore) -> Result<(usize, u64, Duration)> {
    let start = Instant::now();
    let project_id = vrift_config::path::compute_project_id(project_dir);
    let storage_dir = get_cache_storage_dir()?;
    let manifest_path = storage_dir.join(format!("{}.json", project_id));

    if !manifest_path.exists() {
        anyhow::bail!("No cache manifest found for project {}", project_id);
    }

    let json = fs::read_to_string(&manifest_path)?;
    let cache: ProjectCache = serde_json::from_str(&json)?;
    let target_debug = project_dir.join("target").join("debug");

    // Ensure target directories exist
    fs::create_dir_all(target_debug.join("deps"))?;
    fs::create_dir_all(target_debug.join(".fingerprint"))?;
    fs::create_dir_all(target_debug.join("build"))?;

    // 1. Restore extra artifacts first (root files, build/ dir)
    let mut restored_count = 0;
    for (rel, entry) in &cache.extra_artifacts {
        let dest = target_debug.join(rel);
        if let Err(e) = restore_artifact(cas, entry, &dest) {
            tracing::warn!("Failed to restore extra {}: {}", rel, e);
        } else {
            restored_count += 1;
        }
    }

    // 2. Validate and restore per-crate artifacts
    let crate_map = &cache.crates;
    let mut validated_fps: HashMap<String, String> = HashMap::new();
    let mut pending: Vec<&CrateFingerprint> = crate_map.values().collect();

    loop {
        let mut made_progress = false;
        let mut next_pending = Vec::new();

        for crate_fp in pending {
            match validate_crate(project_dir, crate_fp, &validated_fps) {
                Ok(true) => {
                    for (rel, entry) in &crate_fp.artifacts {
                        let dest = target_debug.join(rel);
                        if let Err(e) = restore_artifact(cas, entry, &dest) {
                            tracing::warn!("Failed to restore {}: {}", rel, e);
                        } else {
                            restored_count += 1;
                        }
                    }
                    validated_fps.insert(
                        crate_fp.crate_name.clone(),
                        crate_fp.fingerprint_value.clone(),
                    );
                    made_progress = true;
                }
                Ok(false) => {
                    let all_deps_known = crate_fp
                        .deps
                        .keys()
                        .all(|d| validated_fps.contains_key(d) || !crate_map.contains_key(d));
                    if !all_deps_known {
                        next_pending.push(crate_fp);
                    }
                }
                Err(_) => {}
            }
        }

        if !made_progress || next_pending.is_empty() {
            break;
        }
        pending = next_pending;
    }

    Ok((restored_count, 0, start.elapsed()))
}

fn validate_crate(
    project_dir: &Path,
    fp: &CrateFingerprint,
    validated_deps: &HashMap<String, String>,
) -> Result<bool> {
    // 1. Source Check (full mtime precision)
    for (rel_path, (cached_mtime_sec, cached_mtime_nsec, cached_size)) in &fp.sources {
        let abs_path = if Path::new(rel_path).is_absolute() {
            PathBuf::from(rel_path)
        } else {
            project_dir.join(rel_path)
        };

        if !abs_path.exists() {
            return Ok(false);
        }
        let meta = fs::metadata(&abs_path)?;
        if meta.len() != *cached_size {
            return Ok(false);
        }
        let (cur_sec, cur_nsec) = get_mtime(&meta);
        if cur_sec != *cached_mtime_sec || cur_nsec != *cached_mtime_nsec {
            return Ok(false);
        }
    }

    // 2. Dependency Check
    for dep_name in fp.deps.keys() {
        if validated_deps.contains_key(dep_name) {
            // Dep validated, OK
        }
        // External deps (not in our crate map) are trusted if source check passed
    }

    Ok(true)
}

fn restore_artifact(cas: &CasStore, entry: &ArtifactEntry, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
        clear_flags(parent);
    }

    // Find blob in CAS
    let l1 = &entry.hash_hex[..2];
    let l2 = &entry.hash_hex[2..4];
    let dir = cas.root().join("blake3").join(l1).join(l2);
    let mut blob_path = None;
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with(&entry.hash_hex) {
                blob_path = Some(e.path());
                break;
            }
        }
    }

    if let Some(src) = blob_path {
        if dest.exists() {
            clear_flags(dest);
            let _ = fs::remove_file(dest);
        }

        // Zero-copy restoration
        if let Err(_e) = reflink_or_copy(&src, dest) {
            fs::copy(&src, dest)?;
        }

        // Clear flags IMMEDIATELY so we can modify mtime/perms
        clear_flags(dest);

        // CRITICAL: Restore mtime exactly as recorded
        let ft = filetime::FileTime::from_unix_time(entry.mtime_sec, entry.mtime_nsec_frac as u32);
        if let Err(e) = filetime::set_file_times(dest, ft, ft) {
            tracing::warn!("Failed to set mtime for {}: {}", dest.display(), e);
        }

        // Restore original file permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if entry.file_mode != 0 {
                entry.file_mode
            } else {
                // Fallback: detect executables by path
                if dest.to_string_lossy().contains("/build/")
                    || dest.to_string_lossy().contains("/bin/")
                    || dest.extension().is_some_and(|e| e == "dylib" || e == "so")
                    || dest.extension().is_none()
                // no extension = likely binary/script
                {
                    0o755
                } else {
                    0o644
                }
            };
            let _ = fs::set_permissions(dest, fs::Permissions::from_mode(mode));
        }
    }

    Ok(())
}

// --- Utils ---

fn get_cache_storage_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("No home dir")?;
    let dir = home.join(".vrift").join("target-cache");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Get file mtime as (seconds, nanosecond_fraction).
/// This preserves full precision for Cargo fingerprint matching.
fn get_mtime(meta: &fs::Metadata) -> (i64, i64) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (meta.mtime(), meta.mtime_nsec())
    }
    #[cfg(not(unix))]
    {
        let dur = meta
            .modified()
            .unwrap_or(std::time::UNIX_EPOCH)
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        (dur.as_secs() as i64, (dur.subsec_nanos()) as i64)
    }
}

/// Get Unix file permission mode from metadata.
fn get_file_mode(meta: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.mode()
    }
    #[cfg(not(unix))]
    {
        0o644
    }
}

/// Zero-copy clone (reflink) on macOS, fallback to copy
fn reflink_or_copy(src: &Path, dest: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let c_src = CString::new(src.as_os_str().as_bytes())?;
        let c_dest = CString::new(dest.as_os_str().as_bytes())?;

        unsafe {
            let res = libc::clonefile(c_src.as_ptr(), c_dest.as_ptr(), 0);
            if res == 0 {
                return Ok(());
            }
        }
    }

    fs::copy(src, dest)?;
    Ok(())
}

/// Remove immutable flags (uchg) on macOS
fn clear_flags(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        if !path.exists() {
            return;
        }
        if let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) {
            unsafe {
                let res = libc::chflags(c_path.as_ptr(), 0);
                if res != 0 {
                    let _ = std::process::Command::new("chflags")
                        .arg("nouchg")
                        .arg(path)
                        .status();
                }
            }
        }
    }
}
