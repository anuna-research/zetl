//! Model acquisition for semantic search (REQ-104, REQ-105).
//!
//! Resolves the ONNX model and tokenizer paths, downloading from HuggingFace Hub
//! with user confirmation when files are absent, validating integrity via SHA-256,
//! and caching permanently under `.zetl/models/`.
//!
//! `ZETL_MODEL_PATH` env var overrides the model path entirely (no download,
//! no SHA validation; tokenizer must reside alongside the model).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::MODEL_NAME;

/// Download URL for the ONNX model (sentence-transformers HuggingFace Hub).
const MODEL_ONNX_URL: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model.onnx";

/// Download URL for the HuggingFace tokenizer config.
const TOKENIZER_URL: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json";

/// Resolve the ONNX model and tokenizer paths, downloading if needed.
///
/// # Precedence
/// 1. `ZETL_MODEL_PATH` → use as-is (skip SHA validation; tokenizer must be
///    alongside the model as `{MODEL_NAME}-tokenizer.json`)
/// 2. `.zetl/models/{MODEL_NAME}.onnx` cached in vault → verify SHA sidecar, return
/// 3. File absent or sidecar mismatch → prompt user, download, validate, cache
///
/// Returns `(model_path, tokenizer_path)`.
pub fn ensure_model(vault_root: &Path) -> Result<(PathBuf, PathBuf)> {
    // 1. ZETL_MODEL_PATH override — skip download and SHA validation entirely.
    if let Ok(override_path) = std::env::var("ZETL_MODEL_PATH") {
        let model_path = PathBuf::from(&override_path);
        anyhow::ensure!(
            model_path.exists(),
            "ZETL_MODEL_PATH='{}' does not exist",
            override_path
        );
        let tokenizer_path = model_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!("{MODEL_NAME}-tokenizer.json"));
        anyhow::ensure!(
            tokenizer_path.exists(),
            "Tokenizer not found at '{}'. \
             Expected alongside ZETL_MODEL_PATH as '{MODEL_NAME}-tokenizer.json'.",
            tokenizer_path.display()
        );
        return Ok((model_path, tokenizer_path));
    }

    // 2. Default cache location.
    let models_dir = vault_root.join(".zetl").join("models");
    let model_path = models_dir.join(format!("{MODEL_NAME}.onnx"));
    let tokenizer_path = models_dir.join(format!("{MODEL_NAME}-tokenizer.json"));

    let model_ok = model_path.exists() && sha256_sidecar_valid(&model_path)?;
    let tokenizer_ok = tokenizer_path.exists();

    if model_ok && tokenizer_ok {
        return Ok((model_path, tokenizer_path));
    }

    // 3. Prompt user and download.
    eprintln!();
    eprintln!("zetl needs '{MODEL_NAME}' for semantic search.");
    eprintln!("Files will be downloaded from HuggingFace Hub (~90 MB) and cached at:");
    eprintln!("  {}", models_dir.display());
    eprint!("\nDownload now? [y/N] ");
    std::io::stderr().flush()?;

    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .context("reading user confirmation")?;
    if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
        anyhow::bail!("Model download cancelled. Re-run `zetl index` to try again.");
    }

    std::fs::create_dir_all(&models_dir)
        .with_context(|| format!("creating models directory {}", models_dir.display()))?;

    if !model_ok {
        eprint!("  Downloading {MODEL_NAME}.onnx... ");
        std::io::stderr().flush()?;
        let size = download_and_store(MODEL_ONNX_URL, &model_path)
            .context("downloading ONNX model")?;
        eprintln!("{:.1} MB", size as f64 / 1_048_576.0);
    }

    if !tokenizer_ok {
        eprint!("  Downloading tokenizer.json... ");
        std::io::stderr().flush()?;
        let size = download_and_store(TOKENIZER_URL, &tokenizer_path)
            .context("downloading tokenizer")?;
        eprintln!("{:.1} MB", size as f64 / 1_048_576.0);
    }

    eprintln!("Model ready.");
    Ok((model_path, tokenizer_path))
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Download `url` atomically to `dest`, compute SHA-256, write a `.sha256`
/// sidecar, and return the number of bytes written.
fn download_and_store(url: &str, dest: &Path) -> Result<usize> {
    let resp = ureq::get(url)
        .call()
        .with_context(|| format!("HTTP GET {url}"))?;

    let mut data = Vec::new();
    resp.into_reader()
        .read_to_end(&mut data)
        .with_context(|| format!("reading response body from {url}"))?;

    let len = data.len();

    // Atomic write via a temp file.
    let tmp = dest.with_extension("tmp");
    std::fs::write(&tmp, &data)
        .with_context(|| format!("writing to {}", tmp.display()))?;
    std::fs::rename(&tmp, dest)
        .with_context(|| format!("moving {} to {}", tmp.display(), dest.display()))?;

    // Persist SHA-256 sidecar for future integrity checks.
    let hash_hex = sha256_hex(&data);
    let sidecar = sidecar_path(dest);
    std::fs::write(&sidecar, &hash_hex)
        .with_context(|| format!("writing SHA-256 sidecar {}", sidecar.display()))?;

    Ok(len)
}

/// Return `true` if no sidecar exists (manually placed file — accepted as-is)
/// or if the sidecar matches the file's current SHA-256.
///
/// Returns `Err` when the sidecar exists but the hash does not match, indicating
/// a corrupted or replaced file.
fn sha256_sidecar_valid(model_path: &Path) -> Result<bool> {
    let sidecar = sidecar_path(model_path);
    if !sidecar.exists() {
        // No sidecar: file was placed manually; trust it.
        return Ok(true);
    }
    let expected = std::fs::read_to_string(&sidecar)
        .with_context(|| format!("reading {}", sidecar.display()))?;
    let data = std::fs::read(model_path)
        .with_context(|| format!("reading {}", model_path.display()))?;
    let actual = sha256_hex(&data);
    anyhow::ensure!(
        actual == expected.trim(),
        "SHA-256 mismatch for {}:\n  recorded: {}\n  actual:   {}\n\
         Delete '{}' to trigger a fresh download.",
        model_path.display(),
        expected.trim(),
        actual,
        model_path.display()
    );
    Ok(true)
}

/// Path of the SHA-256 sidecar for `path` (appends `.sha256` extension segment).
fn sidecar_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".sha256");
    PathBuf::from(s)
}

/// Compute lowercase hex SHA-256 of `data`.
fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(data))
}
