use aht_core::model::*;
use aht_core::{
    ConnectionId, GroupId, HarnessProfileId, ProviderId, S3AccountId, SecretId, SnippetId,
};
use aht_vault::*;
use serde_json::json;
use std::path::Path;
use std::time::Duration;

const PW: &str = "correct horse battery staple";

fn fast() -> KdfParams {
    KdfParams::insecure_fast()
}

fn new_vault(dir: &Path) -> (Vault, SecretKey) {
    Vault::create(dir, PW, fast()).unwrap()
}

fn tick() {
    std::thread::sleep(Duration::from_millis(3));
}

fn secret(value: &str) -> Secret {
    Secret {
        id: SecretId::new(),
        kind: SecretKind::Password,
        label: "db".into(),
        value: value.into(),
    }
}

fn connection(name: &str) -> Connection {
    Connection {
        id: ConnectionId::new(),
        name: name.into(),
        kind: ConnectionKind::Ssh,
        group_id: Some(GroupId::new()),
        host: "prod-db-01.example.com".into(),
        port: 22,
        user: "root".into(),
        auth: SshAuth::Password {
            secret: SecretId::new(),
        },
        jump: vec![ConnectionId::new()],
        env: Environment::Prod,
        tags: vec!["db".into()],
        color: Some("#ff0000".into()),
        approval_mode: Some(ApprovalMode::ConfirmAll),
        local_shell: None,
        notes: "primary".into(),
        tunnels: vec![],
        rdp_admin: false,
        icon: None,
    }
}

fn item_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir.join("items"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    v.sort();
    v
}

/// Push every raw file of `from` into `to`.
fn sync(from: &Vault, to: &mut Vault) -> Vec<MergeOutcome> {
    from.raw_files()
        .unwrap()
        .iter()
        .map(|f| to.merge_remote(&f.path, &f.bytes).unwrap())
        .collect()
}

#[test]
fn create_unlock_lock_and_wrong_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let st = Vault::open(dir.path()).unwrap().status();
    assert_eq!(
        st,
        VaultStatus {
            initialized: false,
            locked: true
        }
    );

    let (mut v, sk) = new_vault(dir.path());
    assert!(!v.status().locked);
    assert!(matches!(
        Vault::create(dir.path(), PW, fast()),
        Err(VaultError::AlreadyInitialized)
    ));
    let s = secret("hunter2");
    v.put(&s).unwrap();
    v.lock();
    assert!(v.status().locked);
    assert!(matches!(
        v.get::<Secret>(&s.id.to_string()),
        Err(VaultError::Locked)
    ));
    assert!(matches!(v.put(&s), Err(VaultError::Locked)));

    let mut v = Vault::open(dir.path()).unwrap();
    assert_eq!(
        v.status(),
        VaultStatus {
            initialized: true,
            locked: true
        }
    );
    assert!(matches!(
        v.unlock("wrong", &sk),
        Err(VaultError::WrongCredentials)
    ));
    let other_sk = SecretKey::generate().unwrap();
    assert!(matches!(
        v.unlock(PW, &other_sk),
        Err(VaultError::WrongCredentials)
    ));
    assert!(v.status().locked);
    // Secret Key re-parsed from its printed form works.
    let reparsed = SecretKey::parse(&sk.reveal().to_lowercase()).unwrap();
    v.unlock(PW, &reparsed).unwrap();
    assert_eq!(v.get::<Secret>(&s.id.to_string()).unwrap(), Some(s));
    assert!(v.corrupt_files().is_empty());
}

#[test]
fn manifest_is_plaintext_and_kdf_params_persist() {
    let dir = tempfile::tempdir().unwrap();
    let (v, _) = new_vault(dir.path());
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(m["format_version"], 1);
    assert_eq!(m["kdf_algorithm"], "argon2id");
    assert_eq!(m["kdf"]["m_kib"], 8);
    assert_eq!(m["vault_id"].as_str(), v.vault_id());
    assert_eq!(hex_len(&m["salt"]), 32);
    let d = KdfParams::default();
    assert_eq!((d.m_kib, d.t, d.p), (65536, 3, 1));
}

fn hex_len(v: &serde_json::Value) -> usize {
    v.as_str().unwrap().len()
}

#[test]
fn tampered_manifest_key_check_is_wrong_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let (_, sk) = new_vault(dir.path());
    let p = dir.path().join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    let mut kc = m["key_check"].as_str().unwrap().to_string();
    let last = kc.pop().unwrap();
    kc.push(if last == '0' { '1' } else { '0' });
    m["key_check"] = json!(kc);
    std::fs::write(&p, serde_json::to_vec(&m).unwrap()).unwrap();
    let mut v = Vault::open(dir.path()).unwrap();
    assert!(matches!(
        v.unlock(PW, &sk),
        Err(VaultError::WrongCredentials)
    ));
}

#[test]
fn snippets_round_trip_and_list() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, sk) = new_vault(dir.path());
    let snip = Snippet {
        id: SnippetId::new(),
        name: "Disk usage".into(),
        command: "df -h {{path}}".into(),
        description: "Free space per filesystem".into(),
        tags: vec!["disk".into()],
    };
    v.put(&snip).unwrap();
    drop(v);
    let mut v = Vault::open(dir.path()).unwrap();
    v.unlock(PW, &sk).unwrap();
    assert_eq!(v.list::<Snippet>().unwrap(), vec![snip.clone()]);
    // Older entries without the optional fields still load.
    let bare: Snippet = serde_json::from_value(json!({
        "id": SnippetId::new(), "name": "uptime", "command": "uptime"
    }))
    .unwrap();
    assert!(bare.description.is_empty() && bare.tags.is_empty());
    v.delete::<Snippet>(&snip.id.to_string()).unwrap();
    assert!(v.list::<Snippet>().unwrap().is_empty());
}

#[test]
fn s3_accounts_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, sk) = new_vault(dir.path());
    let acct = S3Account {
        id: S3AccountId::new(),
        name: "Backups".into(),
        endpoint: "https://acct.r2.cloudflarestorage.com".into(),
        region: "auto".into(),
        bucket: Some("backups".into()),
        access_key_id: "AKID".into(),
        secret: Some(SecretId::new()),
        path_style: false,
    };
    v.put(&acct).unwrap();
    drop(v);
    let mut v = Vault::open(dir.path()).unwrap();
    v.unlock(PW, &sk).unwrap();
    assert_eq!(v.list::<S3Account>().unwrap(), vec![acct]);
}

#[test]
fn round_trip_all_item_types_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, sk) = new_vault(dir.path());
    let conn = connection("db");
    let group = Group {
        id: GroupId::new(),
        name: "Prod".into(),
        parent_id: Some(GroupId::new()),
        order: 3,
    };
    let sec = secret("s3cr3t");
    let prov = Provider {
        id: ProviderId::new(),
        name: "DeepSeek".into(),
        kind: ProviderKind::OpenAiCompat,
        preset: ProviderPreset::DeepSeek,
        base_url: "https://api.deepseek.com".into(),
        api_key: Some(SecretId::new()),
        default_model: "deepseek-chat".into(),
    };
    let harness = HarnessProfile {
        id: HarnessProfileId::new(),
        name: "DevOps".into(),
        harness: HarnessKind::ClaudeCode,
        display: HarnessDisplay::Sidebar,
        server_mode: true,
        skills: vec!["/skills/a".into()],
        plugins: vec![],
        extra_args: vec!["--verbose".into()],
        model: Some("opus".into()),
    };
    let setting = Setting {
        key: "approval.default".into(),
        value: json!({"mode": "confirm_all", "deny": ["rm -rf"]}),
    };
    v.put(&conn).unwrap();
    v.put(&group).unwrap();
    v.put(&sec).unwrap();
    v.put(&prov).unwrap();
    v.put(&harness).unwrap();
    v.put(&setting).unwrap();
    // Overwrite keeps a single file per item.
    let mut conn2 = conn.clone();
    conn2.port = 2222;
    v.put(&conn2).unwrap();
    assert_eq!(item_files(dir.path()).len(), 6);

    let device = v.device_id();
    drop(v);
    let mut v = Vault::open(dir.path()).unwrap();
    assert_eq!(v.device_id(), device);
    v.unlock(PW, &sk).unwrap();
    assert_eq!(
        v.get::<Connection>(&conn.id.to_string()).unwrap(),
        Some(conn2.clone())
    );
    assert_eq!(
        v.get::<Group>(&group.id.to_string()).unwrap(),
        Some(group.clone())
    );
    assert_eq!(
        v.get::<Secret>(&sec.id.to_string()).unwrap(),
        Some(sec.clone())
    );
    assert_eq!(
        v.get::<Provider>(&prov.id.to_string()).unwrap(),
        Some(prov.clone())
    );
    assert_eq!(
        v.get::<HarnessProfile>(&harness.id.to_string()).unwrap(),
        Some(harness.clone())
    );
    assert_eq!(v.get::<Setting>("approval.default").unwrap(), Some(setting));
    assert_eq!(v.list::<Connection>().unwrap(), vec![conn2]);
    assert_eq!(v.list::<Group>().unwrap().len(), 1);
    // Same id under a different type is a different item.
    assert_eq!(v.get::<Group>(&conn.id.to_string()).unwrap(), None);
    assert_eq!(v.get::<Secret>("missing").unwrap(), None);
}

#[test]
fn list_is_sorted_and_typed() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, _) = new_vault(dir.path());
    for k in ["b", "a", "c"] {
        v.put(&Setting {
            key: k.into(),
            value: json!(k),
        })
        .unwrap();
    }
    v.put(&secret("x")).unwrap();
    let keys: Vec<String> = v
        .list::<Setting>()
        .unwrap()
        .into_iter()
        .map(|s| s.key)
        .collect();
    assert_eq!(keys, ["a", "b", "c"]);
}

#[test]
fn tampered_item_file_is_detected_not_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, sk) = new_vault(dir.path());
    let a = secret("a");
    let b = secret("b");
    v.put(&a).unwrap();
    v.put(&b).unwrap();
    let raw = v.raw_files().unwrap();
    let item = raw.iter().find(|f| f.path.starts_with("items/")).unwrap();

    // Flip each byte position class: header, nonce, ciphertext, tag.
    for pos in [0, 5, 40, item.bytes.len() - 1] {
        let mut bad = item.bytes.clone();
        bad[pos] ^= 0x01;
        assert!(v.merge_remote(&item.path, &bad).is_err(), "pos {pos}");
    }
    assert!(v.merge_remote(&item.path, &[]).is_err());
    assert!(v.merge_remote(&item.path, &item.bytes[..10]).is_err());

    // A valid file presented under another file's name fails (AAD binds the file id).
    let other = raw
        .iter()
        .find(|f| f.path.starts_with("items/") && f.path != item.path)
        .unwrap();
    assert!(matches!(
        v.merge_remote(&other.path, &item.bytes),
        Err(VaultError::Corrupt(_))
    ));

    // On-disk corruption: unlock still succeeds, the file is reported and skipped.
    let files = item_files(dir.path());
    let mut bytes = std::fs::read(&files[0]).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0x80;
    std::fs::write(&files[0], &bytes).unwrap();
    v.lock();
    v.unlock(PW, &sk).unwrap();
    assert_eq!(v.corrupt_files().len(), 1);
    let live = [a, b]
        .iter()
        .filter(|s| v.get::<Secret>(&s.id.to_string()).unwrap().is_some())
        .count();
    assert_eq!(live, 1);
}

#[test]
fn file_names_and_bytes_reveal_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, _) = new_vault(dir.path());
    let s = secret("TOPSECRETVALUE");
    v.put(&s).unwrap();
    v.put(&Setting {
        key: "theme".into(),
        value: json!("dark"),
    })
    .unwrap();
    let id = s.id.to_string();
    for f in v.raw_files().unwrap() {
        if f.path == "manifest.json" {
            continue;
        }
        let stem = f
            .path
            .strip_prefix("items/")
            .unwrap()
            .strip_suffix(".enc")
            .unwrap();
        assert_eq!(stem.len(), 32);
        assert!(stem.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!f.path.contains(&id) && !f.path.contains("theme"));
        assert!(!f.path.contains(&id.replace('-', "")[..16]));
        let hay = String::from_utf8_lossy(&f.bytes);
        assert!(!hay.contains("TOPSECRETVALUE") && !hay.contains(&id) && !hay.contains("theme"));
        assert!(!hay.contains("secret") && !hay.contains("setting"));
    }
    // Another vault storing the same item id gets a different file name.
    let dir2 = tempfile::tempdir().unwrap();
    let (mut v2, _) = new_vault(dir2.path());
    v2.put(&s).unwrap();
    let names =
        |v: &Vault| -> Vec<String> { v.raw_files().unwrap().into_iter().map(|f| f.path).collect() };
    let n2 = names(&v2);
    assert!(names(&v)
        .iter()
        .all(|n| n == "manifest.json" || !n2.contains(n)));
}

#[test]
fn tombstones_and_compaction() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, sk) = new_vault(dir.path());
    let s = secret("x");
    let id = s.id.to_string();
    v.put(&s).unwrap();
    assert!(v.delete::<Secret>(&id).unwrap());
    assert!(!v.delete::<Secret>(&id).unwrap());
    assert!(!v.delete::<Secret>("never-existed").unwrap());
    assert_eq!(v.get::<Secret>(&id).unwrap(), None);
    assert!(v.list::<Secret>().unwrap().is_empty());
    // Tombstone file still exists for sync.
    assert_eq!(item_files(dir.path()).len(), 1);
    v.lock();
    v.unlock(PW, &sk).unwrap();
    assert_eq!(v.get::<Secret>(&id).unwrap(), None);

    assert_eq!(v.compact_tombstones(Duration::from_secs(3600)).unwrap(), 0);
    assert_eq!(item_files(dir.path()).len(), 1);
    tick();
    assert_eq!(v.compact_tombstones(Duration::ZERO).unwrap(), 1);
    assert!(item_files(dir.path()).is_empty());

    // Re-creating after deletion works.
    v.put(&s).unwrap();
    assert_eq!(v.get::<Secret>(&id).unwrap(), Some(s));
}

#[test]
fn join_and_merge_remote_lww_both_directions_with_history() {
    let da = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap();
    let (mut a, sk) = new_vault(da.path());
    let mut c = connection("v1");
    a.put(&c).unwrap();

    // Device B joins with the synced manifest.
    let manifest = a.raw_files().unwrap().remove(0);
    assert_eq!(manifest.path, "manifest.json");
    let mut b = Vault::join(db.path(), &manifest.bytes).unwrap();
    assert!(matches!(
        Vault::join(db.path(), &manifest.bytes),
        Err(VaultError::AlreadyInitialized)
    ));
    assert!(matches!(
        b.merge_remote("items/00000000000000000000000000000000.enc", b"x"),
        Err(VaultError::Locked)
    ));
    b.unlock(PW, &sk).unwrap();
    assert_ne!(a.device_id(), b.device_id());
    assert_eq!(
        sync(&a, &mut b),
        [MergeOutcome::Unchanged, MergeOutcome::Added]
    );
    assert_eq!(
        sync(&a, &mut b),
        [MergeOutcome::Unchanged, MergeOutcome::Unchanged]
    );
    let id = c.id.to_string();
    assert_eq!(b.get::<Connection>(&id).unwrap(), Some(c.clone()));

    // Concurrent edits: A edits, then B edits later. B's version must win everywhere.
    tick();
    c.name = "edited-on-a".into();
    a.put(&c).unwrap();
    tick();
    let mut cb = c.clone();
    cb.name = "edited-on-b".into();
    b.put(&cb).unwrap();

    // Remote newer → RemoteWon on A; A's version goes to A's history.
    assert_eq!(sync(&b, &mut a)[1], MergeOutcome::RemoteWon);
    assert_eq!(
        a.get::<Connection>(&id).unwrap().unwrap().name,
        "edited-on-b"
    );
    let hist = a.conflict_history().unwrap();
    assert_eq!(hist.len(), 1);
    assert_eq!(hist[0].loser.payload["name"], "edited-on-a");
    assert_eq!(hist[0].loser.device_id, a.device_id());
    assert_eq!(hist[0].winner.device, b.device_id());

    // Remote older → LocalWon on B (B already had its newer edit... simulate by stale file).
    let stale = {
        // Rebuild A's edit by putting an older version into a fresh third device? Simpler:
        // take A's current files before it receives B's next edit.
        tick();
        cb.name = "b-second".into();
        b.put(&cb).unwrap();
        a.raw_files().unwrap()
    };
    let item = stale.iter().find(|f| f.path.starts_with("items/")).unwrap();
    assert_eq!(
        b.merge_remote(&item.path, &item.bytes).unwrap(),
        MergeOutcome::LocalWon
    );
    assert_eq!(b.get::<Connection>(&id).unwrap().unwrap().name, "b-second");
    let hist_b = b.conflict_history().unwrap();
    assert_eq!(hist_b.len(), 1);
    assert_eq!(hist_b[0].loser.payload["name"], "edited-on-b");

    // Tombstones propagate.
    tick();
    assert!(b.delete::<Connection>(&id).unwrap());
    assert_eq!(sync(&b, &mut a)[1], MergeOutcome::RemoteWon);
    assert_eq!(a.get::<Connection>(&id).unwrap(), None);

    // A has observed B's clock: A's next write sorts after B's even without a wall-clock gap.
    c.name = "revived-on-a".into();
    a.put(&c).unwrap();
    assert_eq!(sync(&a, &mut b)[1], MergeOutcome::RemoteWon);
    assert_eq!(
        b.get::<Connection>(&id).unwrap().unwrap().name,
        "revived-on-a"
    );

    a.clear_conflict_history().unwrap();
    assert!(a.conflict_history().unwrap().is_empty());
}

#[test]
fn hlc_ordering_converges_regardless_of_merge_order() {
    let da = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap();
    let (mut a, sk) = new_vault(da.path());
    let manifest = a.raw_files().unwrap().remove(0);
    let mut b = Vault::join(db.path(), &manifest.bytes).unwrap();
    b.unlock(PW, &sk).unwrap();
    // Both write the same setting key with no coordination.
    a.put(&Setting {
        key: "theme".into(),
        value: json!("a"),
    })
    .unwrap();
    b.put(&Setting {
        key: "theme".into(),
        value: json!("b"),
    })
    .unwrap();
    sync(&a, &mut b);
    sync(&b, &mut a);
    let va = a.get::<Setting>("theme").unwrap().unwrap().value;
    let vb = b.get::<Setting>("theme").unwrap().unwrap().value;
    assert_eq!(va, vb);
    // The loser is recorded on at least one side.
    assert!(a.conflict_history().unwrap().len() + b.conflict_history().unwrap().len() >= 1);
}

#[test]
fn merge_rejects_bad_paths_and_foreign_manifests() {
    let da = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap();
    let (mut a, _) = new_vault(da.path());
    let (b, _) = new_vault(db.path());
    for p in [
        "../x",
        "items/zz.enc",
        "items/ABCDEF0123456789ABCDEF0123456789.enc",
        "snapshots/x",
    ] {
        assert!(
            matches!(a.merge_remote(p, b"x"), Err(VaultError::Invalid(_))),
            "{p}"
        );
    }
    let foreign = b.raw_files().unwrap().remove(0);
    assert!(a.merge_remote("manifest.json", &foreign.bytes).is_err());
    assert!(a.merge_remote("manifest.json", b"{not json").is_err());
    // A file from another vault fails authentication.
    let mut b = b;
    b.put(&secret("z")).unwrap();
    let item = b.raw_files().unwrap().pop().unwrap();
    assert!(matches!(
        a.merge_remote(&item.path, &item.bytes),
        Err(VaultError::Corrupt(_))
    ));
    let own = a.raw_files().unwrap().remove(0);
    assert_eq!(
        a.merge_remote("manifest.json", &own.bytes).unwrap(),
        MergeOutcome::Unchanged
    );
    // Windows-style separators are accepted.
    a.put(&secret("w")).unwrap();
    let it = a.raw_files().unwrap().pop().unwrap();
    assert_eq!(
        a.merge_remote(&it.path.replace('/', "\\"), &it.bytes)
            .unwrap(),
        MergeOutcome::Unchanged
    );
}

#[test]
fn raw_files_hashes_and_works_locked() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, _) = new_vault(dir.path());
    v.put(&secret("x")).unwrap();
    v.snapshot().unwrap();
    v.lock();
    let files = v.raw_files().unwrap();
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|f| f.sha256.len() == 64));
    assert!(files
        .iter()
        .all(|f| !f.path.starts_with("snapshots") && !f.path.starts_with("local")));
}

#[test]
fn snapshot_and_restore() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, sk) = new_vault(dir.path());
    let keep = secret("keep");
    let change = secret("original");
    let gone = secret("will-be-deleted");
    v.put(&keep).unwrap();
    v.put(&change).unwrap();
    v.put(&gone).unwrap();
    let snap = v.snapshot().unwrap();
    assert_eq!(v.list_snapshots().unwrap(), vec![snap.clone()]);

    tick();
    let mut changed = change.clone();
    changed.value = "modified".into();
    v.put(&changed).unwrap();
    v.delete::<Secret>(&gone.id.to_string()).unwrap();
    let added = secret("added-later");
    v.put(&added).unwrap();

    v.restore_snapshot(&snap.name).unwrap();
    let mut got = v.list::<Secret>().unwrap();
    got.sort_by_key(|s| s.id);
    let mut want = vec![keep.clone(), change.clone(), gone.clone()];
    want.sort_by_key(|s| s.id);
    assert_eq!(got, want);
    // Safety snapshot of the pre-restore state was taken.
    assert_eq!(v.list_snapshots().unwrap().len(), 2);
    // Restored state persists.
    v.lock();
    v.unlock(PW, &sk).unwrap();
    assert_eq!(v.list::<Secret>().unwrap().len(), 3);
    assert_eq!(v.get::<Secret>(&added.id.to_string()).unwrap(), None);

    assert!(matches!(
        v.restore_snapshot("../manifest.json"),
        Err(VaultError::Invalid(_))
    ));
    assert!(matches!(
        v.restore_snapshot("000000000000000-00000000.snap"),
        Err(VaultError::NotFound(_))
    ));
    // Tampered snapshot is rejected without changes.
    let p = dir.path().join("snapshots").join(&snap.name);
    let mut bytes = std::fs::read(&p).unwrap();
    let n = bytes.len();
    bytes[n - 3] ^= 1;
    std::fs::write(&p, bytes).unwrap();
    assert!(matches!(
        v.restore_snapshot(&snap.name),
        Err(VaultError::Corrupt(_))
    ));
    assert_eq!(v.list::<Secret>().unwrap().len(), 3);
}

#[test]
fn snapshot_rotation_keeps_last_ten() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, _) = new_vault(dir.path());
    v.put(&secret("x")).unwrap();
    let mut names = Vec::new();
    for _ in 0..12 {
        names.push(v.snapshot().unwrap().name);
        tick();
    }
    let kept: Vec<String> = v
        .list_snapshots()
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(kept.len(), MAX_SNAPSHOTS);
    assert_eq!(kept, names[2..]);
}

#[test]
fn tentative_snapshot_can_be_dropped_without_losing_old_ones() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, _) = new_vault(dir.path());
    v.put(&secret("x")).unwrap();
    let mut names = Vec::new();
    for _ in 0..MAX_SNAPSHOTS {
        names.push(v.snapshot().unwrap().name);
        tick();
    }
    // A tentative snapshot does not push the oldest one out...
    let t = v.snapshot_unpruned().unwrap();
    assert_eq!(v.list_snapshots().unwrap().len(), MAX_SNAPSHOTS + 1);
    // ...so dropping it again leaves exactly what was there.
    v.delete_snapshot(&t.name).unwrap();
    let kept: Vec<String> = v
        .list_snapshots()
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(kept, names);
    // Keeping it prunes the oldest.
    tick();
    let t = v.snapshot_unpruned().unwrap();
    v.prune_snapshots().unwrap();
    let kept = v.list_snapshots().unwrap();
    assert_eq!(kept.len(), MAX_SNAPSHOTS);
    assert_eq!(kept.last().unwrap().name, t.name);
    assert_ne!(kept[0].name, names[0]);

    assert!(matches!(
        v.delete_snapshot(&t.name.replace(".snap", "x.snap")),
        Err(VaultError::NotFound(_))
    ));
    assert!(matches!(
        v.delete_snapshot("../manifest.json"),
        Err(VaultError::Invalid(_))
    ));
}

#[test]
fn revision_tracks_writes_and_merges_only() {
    let da = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap();
    let (mut a, sk) = new_vault(da.path());
    let r0 = a.revision();
    let s = secret("one");
    a.put(&s).unwrap();
    let r1 = a.revision();
    assert!(r1 > r0);
    // Reads, snapshots and raw file listing do not count as changes.
    a.get::<Secret>(&s.id.to_string()).unwrap();
    a.list::<Secret>().unwrap();
    a.snapshot().unwrap();
    a.raw_files().unwrap();
    assert_eq!(a.revision(), r1);
    assert!(a.delete::<Secret>(&s.id.to_string()).unwrap());
    assert!(a.revision() > r1);

    let manifest = a.raw_files().unwrap().remove(0);
    let mut b = Vault::join(db.path(), &manifest.bytes).unwrap();
    b.unlock(PW, &sk).unwrap();
    let rb = b.revision();
    sync(&a, &mut b);
    let rb1 = b.revision();
    assert!(rb1 > rb, "a merge that adds files is a change");
    // Merging the same files again changes nothing.
    sync(&a, &mut b);
    assert_eq!(b.revision(), rb1);
}

#[test]
fn restore_conflict_brings_back_the_losing_version() {
    let da = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap();
    let (mut a, sk) = new_vault(da.path());
    let manifest = a.raw_files().unwrap().remove(0);
    let mut b = Vault::join(db.path(), &manifest.bytes).unwrap();
    b.unlock(PW, &sk).unwrap();

    a.put(&Setting {
        key: "theme".into(),
        value: json!("from-a"),
    })
    .unwrap();
    tick();
    b.put(&Setting {
        key: "theme".into(),
        value: json!("from-b"),
    })
    .unwrap();
    // B's edit is newer: merging it into A records A's version as the loser.
    sync(&b, &mut a);
    assert_eq!(
        a.get::<Setting>("theme").unwrap().unwrap().value,
        json!("from-b")
    );
    let rec = a.conflict_history().unwrap().pop().unwrap();
    assert_eq!(rec.loser.payload["value"], json!("from-a"));

    a.restore_conflict(&rec.file_id, rec.recorded_at_ms)
        .unwrap();
    assert_eq!(
        a.get::<Setting>("theme").unwrap().unwrap().value,
        json!("from-a")
    );
    // The restored version is a fresh edit, so it wins on the other device too.
    sync(&a, &mut b);
    assert_eq!(
        b.get::<Setting>("theme").unwrap().unwrap().value,
        json!("from-a")
    );
    assert!(matches!(
        a.restore_conflict(&rec.file_id, rec.recorded_at_ms + 1),
        Err(VaultError::NotFound(_))
    ));
}

#[test]
fn debug_output_redacts_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, sk) = new_vault(dir.path());
    let s = secret("SUPERSECRET");
    v.put(&s).unwrap();
    let dbg = format!("{v:?} {sk:?} {s:?}");
    assert!(!dbg.contains("SUPERSECRET"));
    assert!(!dbg.contains(&sk.reveal()[3..]));
    assert!(dbg.contains("SecretKey(<redacted>)"));

    // History records (which carry payloads) redact too.
    let db = tempfile::tempdir().unwrap();
    let manifest = v.raw_files().unwrap().remove(0);
    let mut b = Vault::join(db.path(), &manifest.bytes).unwrap();
    b.unlock(PW, &sk).unwrap();
    sync(&v, &mut b);
    tick();
    let mut s2 = s.clone();
    s2.value = "SUPERSECRET2".into();
    b.put(&s2).unwrap();
    sync(&b, &mut v);
    let hist = v.conflict_history().unwrap();
    assert_eq!(hist.len(), 1);
    let dbg = format!("{hist:?} {:?}", v.raw_files().unwrap());
    assert!(!dbg.contains("SUPERSECRET"));
}

#[test]
fn file_secret_key_store_unlocks_vault() {
    let dir = tempfile::tempdir().unwrap();
    let (mut v, sk) = new_vault(dir.path());
    let store = FileSecretKeyStore::new(dir.path().join("keys"));
    let vid = v.vault_id().unwrap().to_string();
    store.set(&vid, &sk).unwrap();
    v.lock();
    let loaded = store.get(&vid).unwrap().unwrap();
    v.unlock(PW, &loaded).unwrap();
}

#[test]
fn errors_convert_to_core() {
    let e: aht_core::Error = VaultError::Locked.into();
    assert!(matches!(e, aht_core::Error::VaultLocked));
    let e: aht_core::Error = VaultError::WrongCredentials.into();
    assert!(matches!(e, aht_core::Error::Auth(_)));
}

#[test]
fn erase_removes_everything_and_allows_a_new_vault() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("vault");
    let (mut v, _) = new_vault(&dir);
    v.put(&secret("hunter2")).unwrap();
    v.snapshot().unwrap();
    // A copy an earlier erase could not remove is cleaned up too.
    std::fs::create_dir_all(root.path().join(".vault-deleted-0011").join("items")).unwrap();
    // Locked, as when the password is forgotten.
    v.lock();

    v.erase().unwrap();
    assert_eq!(
        v.status(),
        VaultStatus {
            initialized: false,
            locked: true
        }
    );
    assert!(v.vault_id().is_none());
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "only local/ is recreated"
    );
    let left: Vec<_> = std::fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(left, vec![std::ffi::OsString::from("vault")]);

    // Reopening sees nothing, and a new vault can be created in the same place.
    assert!(!Vault::open(&dir).unwrap().status().initialized);
    let (mut fresh, _) = new_vault(&dir);
    assert!(fresh.list::<Secret>().unwrap().is_empty());
    // Erasing a vault whose directory is gone is fine.
    std::fs::remove_dir_all(&dir).unwrap();
    fresh.erase().unwrap();
    assert!(!fresh.status().initialized);
}
