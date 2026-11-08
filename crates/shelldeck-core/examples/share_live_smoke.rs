//! Opt-in personal Share transport smoke. Credentials arrive on stdin and are
//! never logged. Only dedicated synthetic verification accounts are accepted.
use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use shelldeck_core::config::share::{ShareClient, ShareSource, ShareUpload};
use std::{
    io::{Read, Seek, Write},
    sync::{atomic::AtomicBool, Arc},
};
#[derive(Deserialize)]
struct Credentials {
    manage: String,
    token: String,
    email: String,
}
fn main() -> Result<()> {
    ensure!(
        std::env::var("SHELLDECK_SHARE_LIVE").as_deref() == Ok("1"),
        "Set SHELLDECK_SHARE_LIVE=1 for the synthetic live smoke"
    );
    let credentials: Credentials =
        serde_json::from_reader(std::io::stdin()).context("Invalid stdin credentials")?;
    ensure!(
        credentials.email.starts_with("inklura-desktop-share-qa-")
            && credentials.email.ends_with("@example.invalid"),
        "Synthetic account required"
    );
    let client = ShareClient::new(
        &credentials.manage,
        &credentials.token,
        Arc::new(AtomicBool::new(false)),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let first = client
        .snapshot()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    ensure!(
        first.owner.email == credentials.email,
        "Unexpected account identity"
    );
    let mut checks = Vec::new();
    let mut published = Vec::new();
    let result = (|| -> Result<()> {
        client
            .set_retention(86400)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        ensure!(
            client
                .snapshot()
                .map_err(|error| anyhow::anyhow!(error.to_string()))?
                .default_retention_seconds
                == 86400,
            "Default retention did not persist"
        );
        checks.push("native account snapshot and retention persistence");
        let png = vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31,
            0, 3, 3, 2, 0, 239, 163, 30, 41, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ];
        let upload = ShareUpload {
            filename: "desktop-capture-smoke.png".into(),
            content_type: "image/png".into(),
            source: ShareSource::Bytes(Arc::new(png.clone())),
        };
        let small = client
            .upload(&upload, 300, |_, _| {})
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        published.push(small.id.clone());
        let downloaded = reqwest::blocking::get(&small.url)
            .map_err(|_| anyhow::anyhow!("Capture download failed"))?
            .bytes()
            .map_err(|_| anyhow::anyhow!("Capture read failed"))?;
        ensure!(
            downloaded.as_ref() == png.as_slice(),
            "Capture integrity mismatch"
        );
        checks.push("native image upload and exact byte download");
        let temporary = tempfile::tempdir()?;
        let path = temporary.path().join("desktop-51-mib.zip");
        let length = 51 * 1024 * 1024;
        let mut file = std::fs::File::create(&path)?;
        file.write_all(b"PK\x03\x04")?;
        file.set_len(length)?;
        file.rewind()?;
        let mut source = std::fs::File::open(&path)?;
        let mut expected = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let n = source.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            expected.update(&buffer[..n]);
        }
        let mut progress = 0;
        let large = client
            .upload(
                &ShareUpload::from_path(path)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?,
                300,
                |done, total| {
                    progress = done;
                    assert_eq!(total, length);
                },
            )
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        published.push(large.id.clone());
        ensure!(progress == length, "Incomplete progress");
        let mut download = reqwest::blocking::get(&large.url)
            .map_err(|_| anyhow::anyhow!("Multipart download failed"))?;
        ensure!(
            download.status().is_success(),
            "Multipart download rejected"
        );
        let mut actual = Sha256::new();
        loop {
            let n = download.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            actual.update(&buffer[..n]);
        }
        ensure!(
            actual.finalize() == expected.finalize(),
            "Multipart SHA-256 mismatch"
        );
        checks.push("native 51 MiB multipart upload, bounded progress and full SHA-256");
        let snapshot = client
            .snapshot()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        ensure!(
            snapshot.usage.bytes == first.usage.bytes + length + png.len() as u64,
            "Quota mismatch"
        );
        for id in &published {
            ensure!(
                snapshot.files.iter().any(|file| &file.id == id),
                "Uploaded file missing from native gallery"
            );
        }
        checks.push("personal gallery and quota agree with native uploads");
        Ok(())
    })();
    for id in &published {
        client
            .delete(id)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    }
    if let Err(error) = result {
        bail!(error);
    }
    let last = client
        .snapshot()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    ensure!(
        last.usage.bytes == first.usage.bytes && last.usage.files == first.usage.files,
        "Quota not released"
    );
    checks.push("native deletion restores the original quota");
    println!(
        "{}",
        serde_json::json!({"checks":checks,"no_outbound_mail":true,"charged":false})
    );
    Ok(())
}
