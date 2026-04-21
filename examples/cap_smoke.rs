//! Capability-mode smoke-test harness.
//!
//! Runs `run_capability_build` end-to-end against a single-page
//! fixture, stages the browser shim bundle, and prints the delegated-
//! URL invite plus the dist layout so a downstream tool (ar-crawl
//! session, a browser, curl) can exercise the reader path.
//!
//! Run with:
//!
//!     node src/cap/shim/build.mjs
//!     cargo run --example cap_smoke -- /tmp/cap-smoke/dist
//!
//! The first arg is the dist directory (created / overwritten). The
//! shim bundle at `src/cap/shim/dist/{shim.js,shim.sri,enroll.js,
//! enroll.sri}` must already exist — this harness does not invoke
//! node/esbuild.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand_core::{OsRng, RngCore};

use zetl::cap::build::{run_capability_build, BuildConfig, PageInput, Visibility};
use zetl::cap::derivation::{derive_path_cap, PATH_CAP_DEFAULT_BITS};
use zetl::cap::genkey::{
    build_secret, decode_secret, encode_secret, ParsedSecret, SECRET_VERSION_V1,
};
use zetl::cap::grants::validation::{Grant, GrantMode, GrantsFile};
use zetl::cap::html_shell::{load_shim_integrity, CAPABILITY_SHELL_FILENAME};
use zetl::cap::invite::generate_invite_keypair;
use zetl::cap::recipients::parsing::{
    Cohort, CohortMode, RecipientsFile, VaultSection, AGE_RECIPIENT_V1_PREFIX,
};
use zetl::cap::sign::VaultSigningKey;
use zetl::cap::url_format::CapUrl;

fn main() -> anyhow::Result<()> {
    let out_dir: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/cap-smoke/dist".into())
        .into();

    if out_dir.exists() {
        fs::remove_dir_all(&out_dir)?;
    }
    fs::create_dir_all(&out_dir)?;

    let shim_dist: PathBuf = PathBuf::from("src/cap/shim/dist");

    // ── 1. secrets ─────────────────────────────────────────────
    // `zetl cap genkey`-equivalent: produce a v1 48-byte content secret
    // and a fresh Ed25519 signing key. Both stay in memory — nothing
    // is written to disk.
    let mut random = [0u8; 32];
    OsRng.fill_bytes(&mut random);
    let secret_bytes = build_secret(SECRET_VERSION_V1, &random);
    let secret_b64 = encode_secret(&secret_bytes);
    let secret: ParsedSecret = decode_secret(&secret_b64)?;

    let mut signing_seed = [0u8; 32];
    OsRng.fill_bytes(&mut signing_seed);
    let signing_key = VaultSigningKey::from_bytes(&signing_seed);
    let signing_pubkey_b64 = URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes());

    // Rebuild the shim with our real signing pubkey baked in so
    // signature verification succeeds in the browser.
    let status = std::process::Command::new("node")
        .arg("build.mjs")
        .current_dir("src/cap/shim")
        .env("ZETL_CAP_SIGNING_PUBKEY_B64URL", &signing_pubkey_b64)
        .status()?;
    if !status.success() {
        anyhow::bail!("src/cap/shim/build.mjs failed (status {status:?})");
    }
    let shim_sri = load_shim_integrity(&shim_dist)
        .map_err(|e| anyhow::anyhow!("load shim sri: {e}"))?;
    let enroll_sri_path = shim_dist.join("enroll.sri");
    let enroll_sri = fs::read_to_string(&enroll_sri_path)?.trim().to_string();

    // ── 2. cohort salt + invite keypair ───────────────────────
    let mut cohort_salt = [0u8; 32];
    OsRng.fill_bytes(&mut cohort_salt);
    let cohort_salt_b64 = URL_SAFE_NO_PAD.encode(cohort_salt);

    let invite = generate_invite_keypair(&mut OsRng);
    let invite_pub = invite.public;
    let invite_secret_b64 = invite.secret.into_b64url();

    // ── 3. recipients.toml ─────────────────────────────────────
    let recipients = RecipientsFile {
        version: 1,
        vault: VaultSection {
            signing_pubkey: format!("ed25519:{signing_pubkey_b64}"),
        },
        cohorts: vec![Cohort {
            id: "engineering".into(),
            name: Some("Engineering".into()),
            mode: CohortMode::DelegatedUrl,
            pubkeys: vec![format!(
                "{AGE_RECIPIENT_V1_PREFIX}{}",
                URL_SAFE_NO_PAD.encode(invite_pub)
            )],
            pages: None,
            salt_stable: Some(cohort_salt_b64.clone()),
            salt_rotated: None,
            last_rotated: None,
        }],
    };
    recipients.validate().map_err(|e| anyhow::anyhow!("recipients invalid: {e:?}"))?;

    // ── 4. grants.toml ─────────────────────────────────────────
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let build_epoch = format!(
        "{}",
        chrono_rfc3339(now)
    );
    let grants = GrantsFile {
        version: Some(1),
        grants: vec![Grant {
            id: "g_smoke01".into(),
            cohort: "engineering".into(),
            recipient: format!(
                "{AGE_RECIPIENT_V1_PREFIX}{}",
                URL_SAFE_NO_PAD.encode(invite_pub)
            ),
            mode: GrantMode::DelegatedUrl,
            bound: false,
            name: Some("smoke-reader".into()),
            created: build_epoch.clone(),
            expires: Some(chrono_rfc3339(now + 7 * 86400)),
            revoked: false,
            pages: "*".into(),
        }],
    };

    // ── 5. page content ────────────────────────────────────────
    let page = PageInput {
        slug: "welcome".into(),
        html: "<h1>Welcome</h1><p>This page is end-to-end encrypted.</p>".into(),
        explicit_cohorts: vec![],
    };

    // ── 6. run the build ───────────────────────────────────────
    let config = BuildConfig {
        vault_root: PathBuf::from("/tmp/cap-smoke/vault"),
        out_dir: out_dir.clone(),
        build_epoch: build_epoch.clone(),
        now_unix: now,
        path_cap_bits: PATH_CAP_DEFAULT_BITS,
        visibility: Visibility::Private,
        access: Default::default(),
        shim_integrity: Some(shim_sri.clone()),
        enroll_integrity: Some(enroll_sri.clone()),
        vault_name: "smoke-vault".into(),
        tombstones: Vec::new(),
    };

    let summary = run_capability_build(
        &config,
        &recipients,
        &grants,
        &secret,
        &signing_key,
        &[page],
    )
    .map_err(|e| anyhow::anyhow!("build: {e}"))?;
    eprintln!("{}", summary.stderr_line());

    // ── 7. stage shim.js + enroll.js under assets/ ─────────────
    let assets = out_dir.join("assets");
    fs::create_dir_all(&assets)?;
    fs::copy(shim_dist.join("shim.js"), assets.join("shim.js"))?;
    fs::copy(shim_dist.join("enroll.js"), assets.join("enroll.js"))?;

    // ── 8. emit the invite URL ─────────────────────────────────
    // Driver decodes cohort salt from base64url before feeding HKDF;
    // mirror that so the printed path-cap matches the on-disk directory.
    let cohort_salt_raw = URL_SAFE_NO_PAD.decode(cohort_salt_b64.as_bytes())?;
    let path_cap = derive_path_cap(
        secret.random_body(),
        &cohort_salt_raw,
        "engineering",
        "welcome",
        PATH_CAP_DEFAULT_BITS,
    )?;

    // Tests expect raw path (e.g. 0123ABCD…); compose the invite URL.
    let url = CapUrl::render_delegated(
        "http",
        "127.0.0.1:8787",
        &path_cap,
        "welcome",
        &invite_secret_b64,
    )
    .map_err(|e| anyhow::anyhow!("render_delegated: {e:?}"))?;

    println!("--- cap smoke dist ---");
    println!("dist:           {}", out_dir.display());
    println!("shim.sri:       {shim_sri}");
    println!("signing_pubkey: ed25519:{signing_pubkey_b64}");
    println!("path_cap:       {path_cap}");
    println!("envelope:       {}/c/{path_cap}/welcome.html", out_dir.display());
    println!("shell:          {}/_zetl/{CAPABILITY_SHELL_FILENAME}", out_dir.display());
    println!("invite_url:     {url}");
    println!(
        "NOTE: the browser shell and the envelope share the URL /c/<cap>/<slug>.html;"
    );
    println!(
        "      serve the shell on HTML navigation and the envelope on subsequent fetch."
    );

    // Also dump the URL to a sibling file the next step can slurp.
    fs::write(out_dir.join("_zetl").join("invite-url.txt"), &url)?;

    Ok(())
}

/// Minimal RFC3339 formatter (`YYYY-MM-DDTHH:MM:SSZ`) so the example
/// avoids pulling chrono when it's not a default feature of the crate.
fn chrono_rfc3339(unix_secs: u64) -> String {
    let days_from_epoch = unix_secs / 86400;
    let sec_of_day = unix_secs % 86400;
    let (h, m, s) = (sec_of_day / 3600, (sec_of_day / 60) % 60, sec_of_day % 60);
    let (y, mo, d) = civil_from_days(days_from_epoch as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

// Howard Hinnant's civil_from_days algorithm (MIT), adapted.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z / 146097 } else { (z - 146096) / 146097 };
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

#[allow(dead_code)]
fn _touch(_: &Path) {}
