//! Session-scoped artifacts + inputs for external orchestrators
//! (dev-plan: job-artifacts).
//!
//! The gap this closes: a control plane can dispatch work to many thClaws
//! workers via `POST /agent/run`, but moving the RESULTING FILES between
//! machines had no Bearer-authenticated, job-scoped path — only the
//! workspace-sync surface, which is whole-workspace and trusts the network
//! layer (tunnel / ForwardAuth). Three endpoints under `/v1` fix that:
//!
//! - `GET  /v1/sessions/{sid}/artifacts`        — the run's frozen manifest
//! - `GET  /v1/sessions/{sid}/artifacts/{aid}`  — one snapshotted file
//! - `POST /v1/inputs`                          — place input files into a
//!   workspace before/with a dispatch (prefix-jailed, size-capped)
//!
//! **Atomicity**: `agent_run` accepts `collect_files: ["reports/*.pdf"]`.
//! When the run finishes, matching files are COPIED into
//! `<workspace>/.thclaws/state/artifacts/<session_id>/files/` and hashed —
//! the manifest and the bytes a later GET serves are the snapshot, so a
//! file being edited after the run can never race the download (the old
//! manifest→export two-step could). `state/` is gitignored + pack-stripped.
//!
//! **Least privilege**: everything is scoped to one session id — an
//! orchestrator holding the Bearer token reads that run's declared outputs,
//! not the whole workspace.

use axum::extract::{Path as AxPath, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use super::errors::OpenAiError;
use super::AuthOk;

/// Caps — server-side, non-negotiable (the proposer explicitly asked for
/// server-enforced limits rather than trusting the client).
const MAX_ARTIFACT_FILES: usize = 256;
const MAX_ARTIFACT_TOTAL_BYTES: u64 = 300 * 1024 * 1024; // parity with sync/push
const MAX_INPUT_FILES: usize = 100;
const MAX_INPUT_TOTAL_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const INPUTS_BODY_LIMIT_BYTES: usize = 96 * 1024 * 1024; // base64 overhead over 64MB

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ArtifactEntry {
    pub id: String,
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ArtifactManifest {
    pub session_id: String,
    pub collected_at: String,
    pub patterns: Vec<String>,
    pub artifacts: Vec<ArtifactEntry>,
    /// Files that matched but were skipped by the caps, so a truncated
    /// collection is visible instead of silently partial.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
}

fn artifacts_root(workspace: &Path, session_id: &str) -> PathBuf {
    workspace
        .join(".thclaws")
        .join("state")
        .join("artifacts")
        .join(session_id)
}

/// Session ids come from URL segments — reject anything that isn't the
/// engine's own id shape before it touches a filesystem path.
fn safe_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Snapshot the files matching `patterns` into the session's artifact
/// store and write the frozen manifest. Called at run end (all three
/// agent_run paths). Failures are logged, never fatal to the run.
pub(crate) fn snapshot_artifacts(
    workspace: &Path,
    session_id: &str,
    patterns: &[String],
) -> std::io::Result<ArtifactManifest> {
    let mut builder = globset::GlobSetBuilder::new();
    for p in patterns {
        if let Ok(g) = globset::Glob::new(p) {
            builder.add(g);
        }
    }
    let set = builder
        .build()
        .map_err(|e| std::io::Error::other(format!("bad glob set: {e}")))?;

    let root = artifacts_root(workspace, session_id);
    let files_dir = root.join("files");
    std::fs::create_dir_all(&files_dir)?;

    let mut artifacts: Vec<ArtifactEntry> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut total: u64 = 0;

    for entry in walkdir::WalkDir::new(workspace)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            // Never descend into runtime/VCS/dep trees — artifacts are
            // workspace outputs, and .thclaws would recurse into our own
            // snapshot directory.
            !(name == ".thclaws" || name == ".git" || name == "node_modules")
        })
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = match entry.path().strip_prefix(workspace) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if !set.is_match(rel) {
            continue;
        }
        let rel_str = rel.to_string_lossy().to_string();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if artifacts.len() >= MAX_ARTIFACT_FILES || total + size > MAX_ARTIFACT_TOTAL_BYTES {
            skipped.push(rel_str);
            continue;
        }
        let bytes = match std::fs::read(entry.path()) {
            Ok(b) => b,
            Err(_) => {
                skipped.push(rel_str);
                continue;
            }
        };
        let sha = hex(&Sha256::digest(&bytes));
        let dest = files_dir.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, &bytes)?;
        total += bytes.len() as u64;
        artifacts.push(ArtifactEntry {
            id: format!("a{}", artifacts.len() + 1),
            path: rel_str,
            size: bytes.len() as u64,
            sha256: sha,
        });
    }
    // Deterministic ordering (walkdir order is fs-dependent): sort by
    // path, then re-assign ids so `a1` is stable across identical runs.
    artifacts.sort_by(|a, b| a.path.cmp(&b.path));
    for (i, a) in artifacts.iter_mut().enumerate() {
        a.id = format!("a{}", i + 1);
    }

    let manifest = ArtifactManifest {
        session_id: session_id.to_string(),
        collected_at: chrono::Utc::now().to_rfc3339(),
        patterns: patterns.to_vec(),
        artifacts,
        skipped,
    };
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceQuery {
    pub workspace_dir: Option<String>,
}

fn resolve_workspace(q: &WorkspaceQuery) -> Result<PathBuf, Response> {
    match q
        .workspace_dir
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(raw) => crate::agent_runtime::validate_workspace_dir(raw).map_err(|msg| {
            (
                StatusCode::BAD_REQUEST,
                Json(OpenAiError::invalid_request(msg, "invalid_workspace_dir")),
            )
                .into_response()
        }),
        None => std::env::current_dir().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(OpenAiError::server_error(format!("daemon CWD: {e}"))),
            )
                .into_response()
        }),
    }
}

/// `GET /v1/sessions/{sid}/artifacts` — the frozen manifest.
pub async fn get_manifest(
    _auth: AuthOk,
    AxPath(sid): AxPath<String>,
    Query(q): Query<WorkspaceQuery>,
) -> Result<Response, Response> {
    let ws = resolve_workspace(&q)?;
    if !safe_session_id(&sid) {
        return Err(bad_id());
    }
    let path = artifacts_root(&ws, &sid).join("manifest.json");
    let raw = std::fs::read(&path).map_err(|_| not_found("no artifacts for this session"))?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|_| not_found("manifest unreadable"))?;
    Ok(Json(manifest).into_response())
}

/// `GET /v1/sessions/{sid}/artifacts/{aid}` — one snapshotted file, exactly
/// the bytes that were hashed at collection time.
pub async fn get_artifact(
    _auth: AuthOk,
    AxPath((sid, aid)): AxPath<(String, String)>,
    Query(q): Query<WorkspaceQuery>,
) -> Result<Response, Response> {
    let ws = resolve_workspace(&q)?;
    if !safe_session_id(&sid) {
        return Err(bad_id());
    }
    let root = artifacts_root(&ws, &sid);
    let raw = std::fs::read(root.join("manifest.json"))
        .map_err(|_| not_found("no artifacts for this session"))?;
    let manifest: ArtifactManifest =
        serde_json::from_slice(&raw).map_err(|_| not_found("manifest unreadable"))?;
    let entry = manifest
        .artifacts
        .iter()
        .find(|a| a.id == aid || a.path == aid)
        .ok_or_else(|| not_found("no such artifact id"))?;
    // Serve from the SNAPSHOT — never the live workspace file.
    let file = root.join("files").join(&entry.path);
    let bytes = std::fs::read(&file).map_err(|_| not_found("artifact file missing"))?;
    Ok((
        StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/octet-stream".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"{}\"",
                    entry.path.rsplit('/').next().unwrap_or("artifact")
                ),
            ),
            (
                axum::http::HeaderName::from_static("x-sha256"),
                entry.sha256.clone(),
            ),
        ],
        bytes,
    )
        .into_response())
}

// ── inputs (orchestrator → workspace) ─────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct InputFile {
    /// Workspace-relative path. Must land under an allowed prefix.
    pub path: String,
    /// base64-encoded content.
    pub content_base64: String,
}

#[derive(Debug, Deserialize)]
pub struct InputsRequest {
    pub workspace_dir: Option<String>,
    pub files: Vec<InputFile>,
}

/// Allowed destination prefixes for `POST /v1/inputs`. Default `inputs/`
/// — safe-by-default; the agent reads its inputs from there. Operators
/// widen it with `THCLAWS_INPUTS_PREFIXES="inputs/,src/,docs/"` or open
/// the whole workspace (minus `.thclaws/` + `.git/`) with `*`.
fn allowed_prefixes() -> Vec<String> {
    match std::env::var("THCLAWS_INPUTS_PREFIXES") {
        Ok(v) if !v.trim().is_empty() => v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => vec!["inputs/".to_string()],
    }
}

fn path_allowed(rel: &str, prefixes: &[String]) -> bool {
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.contains("..")
        || rel.starts_with(".thclaws/")
        || rel == ".thclaws"
        || rel.starts_with(".git/")
        || rel == ".git"
    {
        return false;
    }
    prefixes
        .iter()
        .any(|p| p == "*" || rel.starts_with(p.as_str()))
}

/// `POST /v1/inputs` — place files into a workspace ahead of a dispatch.
/// The coder→reviewer handoff: orchestrator downloads worker A's
/// artifacts, POSTs them here to worker B, then `/agent/run`s B.
pub async fn post_inputs(
    _auth: AuthOk,
    Json(req): Json<InputsRequest>,
) -> Result<Response, Response> {
    let ws = resolve_workspace(&WorkspaceQuery {
        workspace_dir: req.workspace_dir.clone(),
    })?;
    if req.files.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(OpenAiError::invalid_request("files[] is empty", "no_files")),
        )
            .into_response());
    }
    if req.files.len() > MAX_INPUT_FILES {
        return Err(limit(format!("more than {MAX_INPUT_FILES} files")));
    }
    let prefixes = allowed_prefixes();
    let mut written: Vec<serde_json::Value> = Vec::new();
    let mut total: usize = 0;
    for f in &req.files {
        if !path_allowed(&f.path, &prefixes) {
            return Err((
                StatusCode::FORBIDDEN,
                Json(OpenAiError::invalid_request(
                    format!(
                        "path '{}' not under an allowed prefix ({}) — set THCLAWS_INPUTS_PREFIXES to widen",
                        f.path,
                        prefixes.join(", ")
                    ),
                    "path_not_allowed",
                )),
            )
                .into_response());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(f.content_base64.as_bytes())
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(OpenAiError::invalid_request(
                        format!("{}: bad base64: {e}", f.path),
                        "bad_base64",
                    )),
                )
                    .into_response()
            })?;
        total += bytes.len();
        if total > MAX_INPUT_TOTAL_BYTES {
            return Err(limit(format!(
                "total decoded size > {MAX_INPUT_TOTAL_BYTES} bytes"
            )));
        }
        let dest = ws.join(&f.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(io_err)?;
        }
        let sha = hex(&Sha256::digest(&bytes));
        std::fs::write(&dest, &bytes).map_err(io_err)?;
        written.push(json!({ "path": f.path, "size": bytes.len(), "sha256": sha }));
    }
    Ok(Json(json!({
        "workspace_dir": ws.display().to_string(),
        "written": written,
    }))
    .into_response())
}

fn bad_id() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(OpenAiError::invalid_request(
            "invalid session id",
            "invalid_session_id",
        )),
    )
        .into_response()
}

fn not_found(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(OpenAiError::invalid_request(msg.to_string(), "not_found")),
    )
        .into_response()
}

fn limit(msg: String) -> Response {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        Json(OpenAiError::invalid_request(msg, "limit_exceeded")),
    )
        .into_response()
}

fn io_err(e: std::io::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(OpenAiError::server_error(format!("io: {e}"))),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_freezes_bytes_and_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        std::fs::create_dir_all(ws.join("reports")).unwrap();
        std::fs::write(ws.join("reports/q3.pdf"), b"PDFBYTES").unwrap();
        std::fs::write(ws.join("notes.txt"), b"not collected").unwrap();

        let m = snapshot_artifacts(ws, "sess-test", &["reports/*.pdf".to_string()]).unwrap();
        assert_eq!(m.artifacts.len(), 1);
        assert_eq!(m.artifacts[0].path, "reports/q3.pdf");
        assert_eq!(m.artifacts[0].id, "a1");

        // Mutate the live file AFTER collection — the snapshot must not change.
        std::fs::write(ws.join("reports/q3.pdf"), b"TAMPERED").unwrap();
        let frozen = std::fs::read(
            artifacts_root(ws, "sess-test")
                .join("files")
                .join("reports/q3.pdf"),
        )
        .unwrap();
        assert_eq!(frozen, b"PDFBYTES");
        let expected_sha = hex(&Sha256::digest(b"PDFBYTES"));
        assert_eq!(m.artifacts[0].sha256, expected_sha);
    }

    #[test]
    fn snapshot_never_recurses_into_thclaws() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        std::fs::create_dir_all(ws.join(".thclaws/state/kms")).unwrap();
        std::fs::write(ws.join(".thclaws/state/kms/secret.md"), b"x").unwrap();
        let m = snapshot_artifacts(ws, "s", &["**/*.md".to_string()]).unwrap();
        assert!(m.artifacts.is_empty());
    }

    #[test]
    fn inputs_path_jail() {
        let p = vec!["inputs/".to_string()];
        assert!(path_allowed("inputs/a.txt", &p));
        assert!(!path_allowed("../etc/passwd", &p));
        assert!(!path_allowed("/abs/path", &p));
        assert!(!path_allowed(".thclaws/settings.json", &p));
        assert!(!path_allowed(".git/config", &p));
        assert!(!path_allowed("src/main.rs", &p));
        let star = vec!["*".to_string()];
        assert!(path_allowed("src/main.rs", &star));
        assert!(!path_allowed(".thclaws/settings.json", &star));
    }

    #[test]
    fn session_id_shape() {
        assert!(safe_session_id("sess-18c05f849ed5b1b8"));
        assert!(!safe_session_id("../../etc"));
        assert!(!safe_session_id(""));
        assert!(!safe_session_id("a/b"));
    }
}
