//! SFTP against a real OpenSSH server. Skipped unless `HANDRAIL_SSH_TEST` is set; CI starts sshd
//! on its Linux runner and sets it:
//!   HANDRAIL_SSH_TEST=user@host:port   HANDRAIL_SSH_TEST_KEY=/path/to/private/key

use aht_ssh::sftp::join;
use aht_ssh::{AuthMaterial, EntryKind, KnownHosts, SshConnection, SshParams, StrictVerifier};
use std::sync::Arc;

fn target() -> Option<(String, String, u16, String)> {
    let t = std::env::var("HANDRAIL_SSH_TEST").ok()?;
    let key = std::fs::read_to_string(std::env::var("HANDRAIL_SSH_TEST_KEY").ok()?).ok()?;
    let (user, rest) = t.split_once('@')?;
    let (host, port) = rest.rsplit_once(':').unwrap_or((rest, "22"));
    Some((user.into(), host.into(), port.parse().ok()?, key))
}

#[tokio::test]
async fn real_sftp_round_trip() {
    let Some((user, host, port, key)) = target() else {
        eprintln!("HANDRAIL_SSH_TEST not set; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let verifier = Arc::new(StrictVerifier {
        store: KnownHosts::new(dir.path().join("known_hosts")),
        accept_new: true,
    });
    let params = SshParams::new(
        host,
        port,
        user,
        AuthMaterial::Key {
            pem: zeroize::Zeroizing::new(key),
            passphrase: None,
        },
    );
    let conn = SshConnection::connect(&params, verifier).await.unwrap();
    let sftp = conn.open_sftp().await.unwrap();

    let home = sftp.canonicalize(".").await.unwrap();
    assert!(home.starts_with('/'), "{home}");
    let remote_dir = join(&home, &format!("handrail-sftp-{}", std::process::id()));
    sftp.mkdir(&remote_dir).await.unwrap();
    assert!(sftp.is_dir(&remote_dir).await.unwrap());

    // 1.5 MB of non-repeating bytes: several transfer chunks.
    let data: Vec<u8> = (0..1_500_000u32).map(|i| (i * 7 + i / 255) as u8).collect();
    let local = dir.path().join("up.bin");
    std::fs::write(&local, &data).unwrap();
    let remote = join(&remote_dir, "up.bin");
    let mut last = (0, 0);
    let sent = sftp
        .upload(&local, &remote, |d, t| last = (d, t))
        .await
        .unwrap();
    assert_eq!(sent, data.len() as u64);
    assert_eq!(last, (sent, sent));

    let listed = sftp.list(&remote_dir).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "up.bin");
    assert_eq!(listed[0].kind, EntryKind::File);
    assert_eq!(listed[0].size, data.len() as u64);

    let renamed = join(&remote_dir, "renamed.bin");
    sftp.rename(&remote, &renamed).await.unwrap();
    let back = dir.path().join("down.bin");
    let got = sftp.download(&renamed, &back, |_, _| {}).await.unwrap();
    assert_eq!(got, data.len() as u64);
    assert_eq!(std::fs::read(&back).unwrap(), data);

    // Symlinks report their target's kind (and size); a dangling one stays a symlink.
    let out = conn
        .exec(&format!(
            "cd '{remote_dir}' && mkdir sub && ln -s renamed.bin file-link && ln -s sub dir-link \
             && ln -s nowhere dead-link"
        ))
        .await
        .unwrap();
    assert_eq!(
        out.exit_status,
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let listed = sftp.list(&remote_dir).await.unwrap();
    let find = |n: &str| listed.iter().find(|e| e.name == n).unwrap().clone();
    let dir_link = find("dir-link");
    assert_eq!((dir_link.kind, dir_link.link), (EntryKind::Dir, true));
    let file_link = find("file-link");
    assert_eq!((file_link.kind, file_link.link), (EntryKind::File, true));
    assert_eq!(file_link.size, data.len() as u64);
    assert_eq!(find("dead-link").kind, EntryKind::Symlink);
    assert!(!find("sub").link);
    assert_eq!(
        listed[0].kind,
        EntryKind::Dir,
        "folders and links to them first"
    );
    // A link is removed as a file, never as the folder it points to.
    for l in ["file-link", "dir-link", "dead-link"] {
        sftp.remove(&join(&remote_dir, l), false).await.unwrap();
    }
    sftp.remove(&join(&remote_dir, "sub"), true).await.unwrap();

    // A missing file is NotFound and leaves no partial local file.
    let missing = dir.path().join("missing.bin");
    let err = sftp
        .download(&join(&remote_dir, "nope"), &missing, |_, _| {})
        .await
        .unwrap_err();
    assert!(matches!(err, aht_core::Error::NotFound(_)), "{err:?}");
    assert!(!missing.exists());

    sftp.remove(&renamed, false).await.unwrap();
    sftp.remove(&remote_dir, true).await.unwrap();
    assert!(sftp.list(&remote_dir).await.is_err());
    sftp.close().await;
}
