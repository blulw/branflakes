//! Sync engine tests against a stateful fake Spotify Web API (mockito with
//! request callbacks), an in-memory database and a temp artwork folder.
//! No real credentials are used.

use super::*;
use crate::db::{library, open_in_memory, playlists, spotify_links as links};
use crate::http::{self, RetryPolicy};
use crate::model::{Source, TrackUpsert};
use crate::providers::spotify::{SpotifyAdapter, SpotifyTokens, SCOPES};
use crate::secrets::{MemoryStore, Secret, SecretStore, SPOTIFY_TOKENS};
use mockito::{Matcher, Server};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// The fake playlist "p1" on Spotify.
#[derive(Debug, Default)]
struct FakeState {
    name: String,
    owner: String,
    collaborative: bool,
    items: Vec<String>,
    snapshot: u32,
    image: Option<String>,
    uploads: Vec<String>,
    upload_status: u16,
    writes: u32,
    renames: Vec<String>,
    /// Bump the snapshot on the N-th metadata read (simulates a concurrent edit).
    bump_on_meta_read: Option<u32>,
    meta_reads: u32,
    /// Report a wrong `total` (simulates a truncated response).
    short_read: bool,
    /// Answer the next N metadata reads with 429 + Retry-After: 0.
    throttle_meta: u32,
    /// Cover bytes served (default: a 1×1 PNG).
    image_bytes: Option<Vec<u8>>,
    image_downloads: u32,
    /// The playlist doesn't exist (GET /playlists/p1 answers 404).
    gone: bool,
    /// The user removed the playlist from their library.
    not_in_library: bool,
    /// GET /me/library/contains answers 500.
    contains_fails: bool,
    /// DELETE /me/library answers 500 (nothing removed).
    remove_fails: bool,
    library_removals: u32,
    /// POST /me/playlists: refused with 403 / creates but then answers 500.
    create_refused: bool,
    create_then_fail: u32,
    creates: u32,
}

type Fake = Arc<Mutex<FakeState>>;

fn item_json(uri: &str) -> Value {
    if let Some(id) = uri.strip_prefix("spotify:track:") {
        json!({"item": {"type": "track", "id": id, "uri": uri, "name": format!("Song {id}"),
                        "artists": [{"name": "Artist"}], "album": {"name": "Album"}, "duration_ms": 1000}})
    } else if let Some(id) = uri.strip_prefix("spotify:episode:") {
        json!({"item": {"type": "episode", "id": id, "uri": uri, "name": "Ep 1"}})
    } else if uri.starts_with("spotify:local:") {
        json!({"is_local": true, "item": {"type": "track", "id": null, "uri": uri, "name": "Local", "is_local": true}})
    } else {
        json!({"item": null})
    }
}

struct Env {
    server: mockito::ServerGuard,
    fake: Fake,
    db: Mutex<rusqlite::Connection>,
    spotify: SpotifyAdapter,
    store: Arc<MemoryStore>,
    art: tempfile::TempDir,
}

impl Env {
    fn ctx(&self) -> SyncCtx<'_> {
        SyncCtx { db: &self.db, spotify: &self.spotify, client_id: "cid".into(), artwork_dir: self.art.path().to_path_buf() }
    }
    fn conn(&self) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
        self.db.lock().unwrap()
    }
    fn remote(&self) -> Vec<String> {
        self.fake.lock().unwrap().items.clone()
    }
    fn writes(&self) -> u32 {
        self.fake.lock().unwrap().writes
    }
    fn local_uris(&self, pid: &str) -> Vec<String> {
        playlists::tracks(&self.conn(), pid).unwrap().into_iter().map(|t| format!("spotify:track:{}", t.provider_id)).collect()
    }
    /// Track id in Branflakes for a Spotify track id (creates it).
    fn spotify_track(&self, id: &str) -> String {
        let t = TrackUpsert {
            provider_id: id.into(),
            provider_uri: Some(format!("spotify:track:{id}")),
            title: format!("Song {id}"),
            ..Default::default()
        };
        library::upsert_track(&self.conn(), Source::Spotify, &t, false).unwrap()
    }
    fn other_track(&self, source: Source, id: &str) -> String {
        let t = TrackUpsert { provider_id: id.into(), title: format!("Other {id}"), ..Default::default() };
        library::upsert_track(&self.conn(), source, &t, true).unwrap()
    }
}

impl Env {
    /// What `playlist_add_tracks` does: reconcile with Spotify, then add/fork.
    async fn add(&self, pid: &str, ids: &[String], pos: Option<usize>) -> AddOutcome {
        add_tracks_reconciled(&self.db, Ok(&self.ctx()), None, pid, ids, pos, Duration::from_secs(5)).await.unwrap()
    }
}

fn tokens(scope: &str, expires_in: i64) -> String {
    serde_json::to_string(&SpotifyTokens {
        access_token: Secret::new("AT"),
        refresh_token: Some(Secret::new("RT")),
        expires_at: chrono::Utc::now().timestamp() + expires_in,
        scope: scope.into(),
    })
    .unwrap()
}

fn uris(s: &str) -> Vec<String> {
    s.split_whitespace().map(|x| if x.contains(':') { x.to_string() } else { format!("spotify:track:{x}") }).collect()
}

async fn env_with(state: FakeState) -> Env {
    let mut server = Server::new_async().await;
    let base = server.url();
    let fake: Fake = Arc::new(Mutex::new(FakeState { upload_status: 202, ..state }));

    server.mock("GET", "/v1/me").with_body(r#"{"id":"me","display_name":"Me"}"#).create_async().await;

    let f = fake.clone();
    let f429 = fake.clone();
    let b2 = base.clone();
    server
        .mock("GET", "/v1/playlists/p1")
        .match_query(Matcher::Any)
        .with_header("retry-after", "0")
        .with_status_code_from_request(move |_| {
            let mut s = f429.lock().unwrap();
            if s.throttle_meta > 0 {
                s.throttle_meta -= 1;
                429
            } else if s.gone {
                404
            } else {
                200
            }
        })
        .with_body_from_request(move |_| {
            let mut s = f.lock().unwrap();
            if s.gone {
                return br#"{"error":{"status":404,"message":"Resource not found"}}"#.to_vec();
            }
            s.meta_reads += 1;
            if s.bump_on_meta_read == Some(s.meta_reads) {
                s.snapshot += 1;
            }
            let images: Vec<Value> = s.image.iter().map(|u| json!({"url": format!("{b2}/v1/img/{u}?ts=1"), "width": 640})).collect();
            serde_json::to_vec(&json!({"id": "p1", "name": s.name, "snapshot_id": format!("snap{}", s.snapshot),
                "collaborative": s.collaborative, "owner": {"id": s.owner}, "images": images}))
            .unwrap()
        })
        .create_async()
        .await;

    let f = fake.clone();
    server
        .mock("GET", "/v1/playlists/p1/items")
        .match_query(Matcher::Any)
        .with_body_from_request(move |_| {
            let s = f.lock().unwrap();
            let items: Vec<Value> = s.items.iter().map(|u| item_json(u)).collect();
            let total = if s.short_read { s.items.len() + 5 } else { s.items.len() };
            serde_json::to_vec(&json!({"items": items, "total": total, "next": null})).unwrap()
        })
        .create_async()
        .await;

    let f = fake.clone();
    server
        .mock("POST", "/v1/playlists/p1/items")
        .with_status(201)
        .with_body_from_request(move |req| {
            let body: Value = serde_json::from_slice(req.body().unwrap()).unwrap();
            let mut s = f.lock().unwrap();
            let add: Vec<String> = body["uris"].as_array().unwrap().iter().map(|u| u.as_str().unwrap().to_string()).collect();
            let at = body["position"].as_u64().map(|p| p as usize).unwrap_or(s.items.len()).min(s.items.len());
            s.items.splice(at..at, add);
            s.snapshot += 1;
            s.writes += 1;
            serde_json::to_vec(&json!({"snapshot_id": format!("snap{}", s.snapshot)})).unwrap()
        })
        .create_async()
        .await;

    let f = fake.clone();
    server
        .mock("DELETE", "/v1/playlists/p1/items")
        .with_body_from_request(move |req| {
            let body: Value = serde_json::from_slice(req.body().unwrap()).unwrap();
            let mut s = f.lock().unwrap();
            let gone: Vec<String> = body["items"].as_array().unwrap().iter().map(|o| o["uri"].as_str().unwrap().to_string()).collect();
            s.items.retain(|u| !gone.contains(u));
            s.snapshot += 1;
            s.writes += 1;
            serde_json::to_vec(&json!({"snapshot_id": format!("snap{}", s.snapshot)})).unwrap()
        })
        .create_async()
        .await;

    let f = fake.clone();
    server
        .mock("PUT", "/v1/playlists/p1/items")
        .with_body_from_request(move |req| {
            let body: Value = serde_json::from_slice(req.body().unwrap()).unwrap();
            let mut s = f.lock().unwrap();
            if let Some(u) = body["uris"].as_array() {
                s.items = u.iter().map(|x| x.as_str().unwrap().to_string()).collect();
            } else {
                let mut list = s.items.clone();
                super::merge::apply(
                    &mut list,
                    &Op::Move { range_start: body["range_start"].as_u64().unwrap() as usize, insert_before: body["insert_before"].as_u64().unwrap() as usize },
                );
                s.items = list;
            }
            s.snapshot += 1;
            s.writes += 1;
            serde_json::to_vec(&json!({"snapshot_id": format!("snap{}", s.snapshot)})).unwrap()
        })
        .create_async()
        .await;

    let f = fake.clone();
    server
        .mock("PUT", "/v1/playlists/p1")
        .with_body_from_request(move |req| {
            let body: Value = serde_json::from_slice(req.body().unwrap()).unwrap();
            let mut s = f.lock().unwrap();
            s.name = body["name"].as_str().unwrap().to_string();
            let n = s.name.clone();
            s.renames.push(n);
            s.snapshot += 1;
            s.writes += 1;
            Vec::new()
        })
        .create_async()
        .await;

    let f = fake.clone();
    let f2 = fake.clone();
    server
        .mock("PUT", "/v1/playlists/p1/images")
        .with_status_code_from_request(move |_| f2.lock().unwrap().upload_status as usize)
        .with_body_from_request(move |req| {
            let mut s = f.lock().unwrap();
            s.writes += 1;
            if s.upload_status < 300 {
                s.uploads.push(req.utf8_lossy_body().unwrap().to_string());
                s.image = Some(format!("up{}", s.uploads.len()));
                Vec::new()
            } else {
                br#"{"error":{"status":403,"message":"Forbidden"}}"#.to_vec()
            }
        })
        .create_async()
        .await;

    let f = fake.clone();
    server
        .mock("GET", Matcher::Regex(r"^/v1/img/".into()))
        .with_body_from_request(move |_| {
            let mut s = f.lock().unwrap();
            s.image_downloads += 1;
            s.image_bytes.clone().unwrap_or_else(|| PNG_1X1.to_vec())
        })
        .create_async()
        .await;

    // ---- library and playlist lifecycle
    let f = fake.clone();
    let f2 = fake.clone();
    server
        .mock("GET", "/v1/me/library/contains")
        .match_query(Matcher::UrlEncoded("uris".into(), "spotify:playlist:p1".into()))
        .with_status_code_from_request(move |_| if f2.lock().unwrap().contains_fails { 500 } else { 200 })
        .with_body_from_request(move |_| {
            let s = f.lock().unwrap();
            serde_json::to_vec(&json!([!s.gone && !s.not_in_library])).unwrap()
        })
        .create_async()
        .await;
    let f = fake.clone();
    server
        .mock("DELETE", "/v1/me/library")
        .match_query(Matcher::UrlEncoded("uris".into(), "spotify:playlist:p1".into()))
        .with_status_code_from_request(move |_| {
            let mut s = f.lock().unwrap();
            if s.remove_fails {
                return 500;
            }
            s.not_in_library = true;
            s.library_removals += 1;
            200
        })
        .create_async()
        .await;
    let f = fake.clone();
    let f2 = fake.clone();
    server
        .mock("POST", "/v1/me/playlists")
        .with_status_code_from_request(move |req| {
            let mut s = f.lock().unwrap();
            if s.create_refused {
                return 403;
            }
            let body: Value = serde_json::from_slice(req.body().unwrap()).unwrap();
            assert_eq!(body["public"], json!(false), "created as private");
            s.creates += 1;
            s.gone = false;
            s.not_in_library = false;
            s.name = body["name"].as_str().unwrap().to_string();
            s.owner = "me".into();
            s.items.clear();
            s.snapshot += 1;
            if s.create_then_fail > 0 {
                s.create_then_fail -= 1;
                return 500; // created, but the answer is lost
            }
            201
        })
        .with_body_from_request(move |_| {
            let s = f2.lock().unwrap();
            serde_json::to_vec(&json!({"id": "p1", "snapshot_id": format!("snap{}", s.snapshot)})).unwrap()
        })
        .create_async()
        .await;
    let f = fake.clone();
    server
        .mock("GET", "/v1/me/playlists")
        .match_query(Matcher::Any)
        .with_body_from_request(move |_| {
            let s = f.lock().unwrap();
            let items: Vec<Value> = if s.gone || s.not_in_library {
                vec![]
            } else {
                vec![json!({"id": "p1", "name": s.name, "owner": {"id": s.owner}, "collaborative": s.collaborative, "items": {"total": s.items.len()}})]
            };
            serde_json::to_vec(&json!({"items": items, "next": null})).unwrap()
        })
        .create_async()
        .await;

    let store = Arc::new(MemoryStore::default());
    store.set(SPOTIFY_TOKENS, &tokens(&SCOPES.join(" "), 3600)).unwrap();
    let spotify = SpotifyAdapter::with_base(http::client(), store.clone(), &format!("{base}/v1"), &base, RetryPolicy::immediate());
    Env { server, fake, db: Mutex::new(open_in_memory().unwrap()), spotify, store, art: tempfile::tempdir().unwrap() }
}

fn owned(items: &str) -> FakeState {
    FakeState { name: "Road trip".into(), owner: "me".into(), items: uris(items), snapshot: 1, ..Default::default() }
}

// ----------------------------------------------------------- enabling

#[tokio::test]
async fn enable_imports_contents_name_artwork_and_records_snapshot() {
    let e = env_with(FakeState { image: Some("cover1".into()), ..owned("a b c") }).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let p = playlists::get(&e.conn(), &pid).unwrap();
    assert_eq!(p.name, "Road trip");
    assert_eq!(p.origin.as_deref(), Some("spotify:playlist:p1"));
    assert!(p.artwork_path.as_deref().is_some_and(|a| std::path::Path::new(a).exists()), "cover downloaded into the cache");
    let sync = p.sync.expect("marked Spotify Synced");
    assert_eq!(sync.status, "ok");
    assert!(sync.last_synced_at.is_some());
    assert_eq!(e.local_uris(&pid), uris("a b c"));
    let link = links::get(&e.conn(), &pid).unwrap().unwrap();
    assert_eq!(link.snapshot_id.as_deref(), Some("snap1"));
    assert_eq!(e.writes(), 0, "enabling never writes to Spotify");
    // A sync right after enabling has nothing to do.
    let r = sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!((r.pushed_ops, r.pulled_added, r.pulled_removed), (0, 0, 0));
}

#[tokio::test]
async fn enable_reuses_an_existing_imported_copy() {
    let e = env_with(owned("a b")).await;
    let old = {
        let conn = e.conn();
        playlists::create(&conn, "Old name", Some("spotify:playlist:p1")).unwrap().id
    };
    let stale = e.spotify_track("zzz");
    playlists::set_items(&e.conn(), &old, &[stale]).unwrap();
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    assert_eq!(pid, old);
    assert_eq!(e.local_uris(&pid), uris("a b"), "Spotify is the starting point (matches previous re-import behaviour)");
}

#[tokio::test]
async fn enable_refuses_playlists_the_user_cannot_modify() {
    let e = env_with(FakeState { owner: "someone-else".into(), ..owned("a") }).await;
    let err = enable(&e.ctx(), "p1").await.unwrap_err();
    assert!(matches!(&err, AppError::Unsupported(m) if m.contains("belongs to another Spotify user")), "{err}");
    assert!(playlists::list(&e.conn()).unwrap().is_empty());

    // Collaborative: items sync, but details can't be edited.
    let e = env_with(FakeState { owner: "someone-else".into(), collaborative: true, ..owned("a") }).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    assert!(!playlists::get(&e.conn(), &pid).unwrap().sync.unwrap().can_edit_details);
}

#[tokio::test]
async fn enable_requires_playlist_write_scopes() {
    let e = env_with(owned("a")).await;
    e.store.set(SPOTIFY_TOKENS, &tokens("playlist-read-private", 3600)).unwrap();
    e.spotify.disconnect().await.unwrap();
    e.store.set(SPOTIFY_TOKENS, &tokens("playlist-read-private", 3600)).unwrap();
    let err = enable(&e.ctx(), "p1").await.unwrap_err();
    assert!(matches!(&err, AppError::Auth { message, .. } if message.contains("Reconnect Spotify")), "{err}");
}

// ------------------------------------------------------- track changes

#[tokio::test]
async fn branflakes_addition_is_pushed_once_and_no_loop() {
    let e = env_with(owned("a b")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let c = e.spotify_track("c");
    let out = add_tracks(&e.conn(), &pid, &[c], Some(1), None).unwrap();
    assert_eq!(out, AddOutcome::Added { added: 1, skipped_duplicates: 0 });
    let r = sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(r.pushed_ops, 1);
    assert_eq!(e.remote(), uris("a c b"), "added at the intended position");
    assert_eq!(e.local_uris(&pid), uris("a c b"));

    // Repeated syncs change nothing and send no writes (no sync loop).
    let writes = e.writes();
    for _ in 0..3 {
        let r = sync_playlist(&e.ctx(), &pid, false).await.unwrap();
        assert_eq!((r.pushed_ops, r.pulled_added, r.pulled_removed), (0, 0, 0));
    }
    assert_eq!(e.writes(), writes);
    assert_eq!(e.remote(), uris("a c b"));
}

#[tokio::test]
async fn spotify_addition_is_imported_in_spotify_order() {
    let e = env_with(owned("a b")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    {
        let mut s = e.fake.lock().unwrap();
        s.items.insert(1, "spotify:track:d".into());
        s.snapshot += 1;
    }
    let r = sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(r.pulled_added, 1);
    assert_eq!(r.pushed_ops, 0, "an imported change is not pushed back");
    assert_eq!(e.local_uris(&pid), uris("a d b"));
    assert_eq!(e.writes(), 0);
}

#[tokio::test]
async fn removals_sync_in_both_directions() {
    let e = env_with(owned("a b c")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    playlists::remove_at(&e.conn(), &pid, 1).unwrap(); // remove b in Branflakes
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(e.remote(), uris("a c"));
    {
        let mut s = e.fake.lock().unwrap();
        s.items.retain(|u| u != "spotify:track:a");
        s.snapshot += 1;
    }
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(e.local_uris(&pid), uris("c"));
}

#[tokio::test]
async fn both_sides_edited_between_syncs_keeps_everything() {
    let e = env_with(owned("a b")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let x = e.spotify_track("x");
    add_tracks(&e.conn(), &pid, &[x], None, None).unwrap();
    {
        let mut s = e.fake.lock().unwrap();
        s.items.insert(0, "spotify:track:y".into());
        s.snapshot += 1;
    }
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(e.remote(), uris("y a b x"));
    assert_eq!(e.local_uris(&pid), uris("y a b x"));
}

#[tokio::test]
async fn duplicates_are_avoided() {
    let e = env_with(owned("a b")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let a = e.spotify_track("a");
    let out = add_tracks(&e.conn(), &pid, &[a.clone(), a.clone()], None, None).unwrap();
    assert_eq!(out, AddOutcome::Added { added: 0, skipped_duplicates: 2 });
    // Same new track added in both apps: kept once.
    let z = e.spotify_track("z");
    add_tracks(&e.conn(), &pid, &[z], None, None).unwrap();
    {
        let mut s = e.fake.lock().unwrap();
        s.items.push("spotify:track:z".into());
        s.snapshot += 1;
    }
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(e.remote(), uris("a b z"));
    assert_eq!(e.local_uris(&pid), uris("a b z"));
}

#[tokio::test]
async fn reorder_in_branflakes_is_pushed() {
    let e = env_with(owned("a b c d")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    playlists::move_item(&e.conn(), &pid, 3, 0).unwrap();
    let r = sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(r.pushed_ops, 1);
    assert_eq!(e.remote(), uris("d a b c"));
}

#[tokio::test]
async fn spotify_local_files_are_preserved() {
    let e = env_with(owned("a spotify:local:x:y:z:1 b")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    assert_eq!(e.local_uris(&pid), uris("a b"));
    let c = e.spotify_track("c");
    add_tracks(&e.conn(), &pid, &[c], None, None).unwrap();
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(e.remote(), uris("a spotify:local:x:y:z:1 b c"));
}

// ---------------------------------------------------- names & artwork

#[tokio::test]
async fn renames_sync_both_ways_and_collaborators_cannot_rename() {
    let e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    playlists::rename(&e.conn(), &pid, "Renamed here").unwrap();
    let r = sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert!(!r.renamed, "local name was already right");
    assert_eq!(e.fake.lock().unwrap().name, "Renamed here");
    {
        let mut s = e.fake.lock().unwrap();
        s.name = "Renamed on Spotify".into();
        s.snapshot += 1;
    }
    let r = sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert!(r.renamed);
    assert_eq!(playlists::get(&e.conn(), &pid).unwrap().name, "Renamed on Spotify");
    assert_eq!(e.fake.lock().unwrap().renames, vec!["Renamed here".to_string()], "no rename echo");

    let e = env_with(FakeState { owner: "other".into(), collaborative: true, ..owned("a") }).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    playlists::rename(&e.conn(), &pid, "Mine now").unwrap();
    let r = sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert!(r.notices.iter().any(|n| n.contains("Only the playlist owner")));
    assert!(e.fake.lock().unwrap().renames.is_empty());
    assert_eq!(playlists::get(&e.conn(), &pid).unwrap().name, "Road trip");
}

#[tokio::test]
async fn artwork_syncs_both_ways_without_redundant_downloads() {
    let e = env_with(FakeState { image: Some("cover1".into()), ..owned("a") }).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let first = playlists::get(&e.conn(), &pid).unwrap().artwork_path.unwrap();

    // Spotify cover changes -> downloaded once.
    {
        let mut s = e.fake.lock().unwrap();
        s.image = Some("cover2".into());
    }
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    let link = links::get(&e.conn(), &pid).unwrap().unwrap();
    assert!(link.base_image_key.unwrap().ends_with("/v1/img/cover2"), "query string ignored in the identity");
    assert!(playlists::get(&e.conn(), &pid).unwrap().artwork_path.is_some());
    let _ = first;

    // Branflakes cover changes -> uploaded as base64 JPEG.
    let img = e.art.path().join("mine.png");
    image::RgbImage::from_pixel(800, 800, image::Rgb([200, 30, 30])).save(&img).unwrap();
    playlists::set_artwork(&e.conn(), &pid, Some(img.to_str().unwrap())).unwrap();
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    let uploads = e.fake.lock().unwrap().uploads.clone();
    assert_eq!(uploads.len(), 1);
    assert!(uploads[0].len() <= crate::providers::spotify::COVER_MAX_BASE64);
    use base64::Engine;
    let jpeg = base64::engine::general_purpose::STANDARD.decode(&uploads[0]).unwrap();
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "JPEG");
    // Spotify now reports the uploaded image under a new URL: recognised as
    // ours, so the Branflakes cover is not replaced by a re-download.
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(playlists::get(&e.conn(), &pid).unwrap().artwork_path.as_deref(), img.to_str());
    assert_eq!(e.fake.lock().unwrap().uploads.len(), 1, "no re-upload");
}

#[tokio::test]
async fn failed_cover_upload_keeps_the_branflakes_cover_and_is_not_retried_automatically() {
    let e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    e.fake.lock().unwrap().upload_status = 403;
    let img = e.art.path().join("mine.png");
    image::RgbImage::from_pixel(64, 64, image::Rgb([1, 2, 3])).save(&img).unwrap();
    playlists::set_artwork(&e.conn(), &pid, Some(img.to_str().unwrap())).unwrap();
    let r = sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert!(r.notices.iter().any(|n| n.contains("couldn't be uploaded")), "{:?}", r.notices);
    assert_eq!(playlists::get(&e.conn(), &pid).unwrap().artwork_path.as_deref(), img.to_str(), "custom cover preserved");
    let attempts = e.writes();
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(e.writes(), attempts, "not retried by automatic sync");
    sync_playlist(&e.ctx(), &pid, true).await.unwrap();
    assert_eq!(e.writes(), attempts + 1, "manual sync retries");
}

// ------------------------------------------------- failures & safety

#[tokio::test]
async fn incomplete_spotify_read_never_removes_anything() {
    let e = env_with(owned("a b c")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    {
        let mut s = e.fake.lock().unwrap();
        s.items.truncate(1);
        s.short_read = true;
        s.snapshot += 1;
    }
    let err = sync_playlist(&e.ctx(), &pid, false).await.unwrap_err();
    assert!(err.to_string().contains("nothing was changed"), "{err}");
    assert_eq!(e.local_uris(&pid), uris("a b c"));
    let p = playlists::get(&e.conn(), &pid).unwrap();
    assert_eq!(p.sync.unwrap().status, "error");
}

#[tokio::test]
async fn concurrent_spotify_edit_during_sync_is_retried_not_overwritten() {
    let e = env_with(owned("a b")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let c = e.spotify_track("c");
    add_tracks(&e.conn(), &pid, &[c], None, None).unwrap();
    // The 2nd metadata read (the pre-push check) sees a new snapshot.
    {
        let mut s = e.fake.lock().unwrap();
        s.meta_reads = 0;
        s.bump_on_meta_read = Some(2);
    }
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(e.remote(), uris("a b c"));
}

#[tokio::test]
async fn rate_limit_quota_is_reported_and_backs_off() {
    let mut e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let _m = e
        .server
        .mock("GET", "/v1/playlists/p1")
        .match_query(Matcher::Any)
        .with_status(429)
        .with_header("retry-after", "7")
        .with_body(r#"{"error":{"status":429,"message":"Too many requests","reason":"QUOTA_EXCEEDED"}}"#)
        .create_async()
        .await;
    let err = sync_playlist(&e.ctx(), &pid, false).await.unwrap_err();
    assert!(matches!(err, AppError::RateLimited { quota_exceeded: true, .. }));
    assert_eq!(backoff_for(&err, Duration::from_secs(300)), Some(Duration::from_secs(3600)));
    let plain = AppError::RateLimited { provider: "spotify".into(), retry_after_secs: Some(7), quota_exceeded: false };
    assert_eq!(backoff_for(&plain, Duration::from_secs(300)), Some(Duration::from_secs(7)));
    assert_eq!(playlists::get(&e.conn(), &pid).unwrap().sync.unwrap().status, "error");
}

#[tokio::test]
async fn retry_after_429_then_success() {
    let e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    // One transient 429 with Retry-After: the HTTP layer waits and retries.
    e.fake.lock().unwrap().throttle_meta = 1;
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(e.fake.lock().unwrap().throttle_meta, 0);
    assert_eq!(playlists::get(&e.conn(), &pid).unwrap().sync.unwrap().status, "ok");
}

#[tokio::test]
async fn expired_sign_in_is_reported_and_changes_nothing() {
    let mut e = env_with(owned("a b")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    e.spotify.disconnect().await.unwrap();
    e.store.set(SPOTIFY_TOKENS, &tokens(&SCOPES.join(" "), -100)).unwrap();
    e.server
        .mock("POST", "/api/token")
        .with_status(400)
        .with_body(r#"{"error":"invalid_grant","error_description":"Refresh token revoked"}"#)
        .create_async()
        .await;
    let err = sync_playlist(&e.ctx(), &pid, false).await.unwrap_err();
    assert!(matches!(err, AppError::Auth { .. }), "{err:?}");
    assert_eq!(e.local_uris(&pid), uris("a b"));
    let s = playlists::get(&e.conn(), &pid).unwrap().sync.unwrap();
    assert_eq!(s.status, "error");
    assert!(s.last_error.unwrap().contains("Refresh token revoked"));
}

#[tokio::test]
async fn offline_sync_fails_safely_and_resumes() {
    let e = env_with(owned("a b")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let c = e.spotify_track("c");
    add_tracks(&e.conn(), &pid, &[c], None, None).unwrap();
    // Point a second adapter at a port nobody listens on.
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        format!("http://127.0.0.1:{port}")
    };
    let offline = SpotifyAdapter::with_base(http::client(), e.store.clone(), &format!("{dead}/v1"), &dead, RetryPolicy::immediate());
    let ctx = SyncCtx { db: &e.db, spotify: &offline, client_id: "cid".into(), artwork_dir: e.art.path().into() };
    let err = sync_playlist(&ctx, &pid, false).await.unwrap_err();
    assert!(matches!(err, AppError::Network { .. }), "{err:?}");
    assert_eq!(backoff_for(&err, Duration::from_secs(300)), Some(Duration::from_secs(60)));
    assert_eq!(e.local_uris(&pid), uris("a b c"), "local edit kept while offline");
    // Connectivity returns: the pending addition goes through.
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(e.remote(), uris("a b c"));
}

#[tokio::test]
async fn restart_after_partial_push_does_not_duplicate() {
    // Simulates a crash after Spotify accepted the push but before the base
    // was saved: Spotify already has c, Branflakes has c, base doesn't.
    let e = env_with(owned("a b")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let c = e.spotify_track("c");
    add_tracks(&e.conn(), &pid, &[c], None, None).unwrap();
    links::set_status(&e.conn(), &pid, "syncing", None).unwrap();
    {
        let mut s = e.fake.lock().unwrap();
        s.items.push("spotify:track:c".into());
        s.snapshot += 1;
    }
    // Startup recovery marks it pending; the scheduler then syncs it.
    assert_eq!(links::recover_interrupted(&e.conn()).unwrap(), vec![pid.clone()]);
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(e.remote(), uris("a b c"));
    assert_eq!(e.local_uris(&pid), uris("a b c"));
}

#[tokio::test]
async fn lost_permission_is_reported() {
    let e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    {
        let mut s = e.fake.lock().unwrap();
        s.owner = "new-owner".into();
    }
    let err = sync_playlist(&e.ctx(), &pid, false).await.unwrap_err();
    assert!(err.to_string().contains("belongs to another Spotify user"));
    assert_eq!(e.local_uris(&pid), uris("a"));
}

// ------------------------------------------------- non-Spotify tracks

async fn fork_case(source: Source) {
    let e = env_with(owned("a b c")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let other = e.other_track(source, "x1");
    let writes = e.writes();
    let base_before = links::get(&e.conn(), &pid).unwrap().unwrap();
    let out = e.add(&pid, &[other.clone()], Some(1)).await;
    let AddOutcome::Forked { playlist: fork, added: 1, not_on_spotify: 1, uncopyable, from_latest: true, note } = out else {
        panic!("expected a fork, got {out:?}")
    };
    assert!(uncopyable.is_empty());
    assert!(note.contains("current version"), "{note}");
    assert!(!note.contains("not complete"));
    // The reconcile was read-only: sync state of the original is unchanged.
    assert_eq!(links::get(&e.conn(), &pid).unwrap().unwrap(), base_before);

    // Original untouched, locally and on Spotify.
    assert_eq!(e.local_uris(&pid), uris("a b c"));
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(e.remote(), uris("a b c"));
    assert_eq!(e.writes(), writes, "nothing was sent to Spotify");

    // Fork: every original track plus the new one at the intended position.
    let ids = playlists::track_ids(&e.conn(), &fork.id).unwrap();
    let orig = playlists::track_ids(&e.conn(), &pid).unwrap();
    assert_eq!(ids, vec![orig[0].clone(), other.clone(), orig[1].clone(), orig[2].clone()]);
    assert_eq!(fork.name, "Road trip (Branflakes)");
    assert!(fork.sync.is_none() && fork.origin.is_none(), "not synced, not imported");
    assert_eq!(fork.forked_from.as_deref(), Some(pid.as_str()));
    assert!(links::get(&e.conn(), &fork.id).unwrap().is_none());
    assert!(matches!(sync_playlist(&e.ctx(), &fork.id, false).await, Err(AppError::NotFound(_))));
    // Independently editable.
    playlists::remove_at(&e.conn(), &fork.id, 0).unwrap();
    assert_eq!(playlists::track_ids(&e.conn(), &pid).unwrap().len(), 3);
}

#[tokio::test]
async fn adding_a_local_track_creates_a_branflakes_only_copy() {
    fork_case(Source::Local).await;
}

#[tokio::test]
async fn adding_a_soundcloud_track_creates_a_branflakes_only_copy() {
    fork_case(Source::Soundcloud).await;
}

#[tokio::test]
async fn mixed_multi_select_goes_entirely_into_the_copy() {
    let e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let s = e.spotify_track("s");
    let l = e.other_track(Source::Local, "/l");
    let out = e.add(&pid, &[s.clone(), l.clone()], None).await;
    let AddOutcome::Forked { playlist, added: 2, not_on_spotify: 1, .. } = out else { panic!("{out:?}") };
    assert_eq!(playlists::track_ids(&e.conn(), &playlist.id).unwrap().len(), 3);
    assert_eq!(e.local_uris(&pid), uris("a"));
}

#[test]
fn ordinary_playlists_are_unaffected() {
    let conn = open_in_memory().unwrap();
    let p = playlists::create(&conn, "Mine", None).unwrap();
    let t = TrackUpsert { provider_id: "/x".into(), title: "X".into(), ..Default::default() };
    let l = library::upsert_track(&conn, Source::Local, &t, true).unwrap();
    assert_eq!(add_tracks(&conn, &p.id, &[l.clone(), l.clone()], None, None).unwrap(), AddOutcome::Added { added: 2, skipped_duplicates: 0 });
    assert_eq!(playlists::list(&conn).unwrap().len(), 1, "no copy created");
    assert!(playlists::get(&conn, &p.id).unwrap().sync.is_none());
}

// ----------------------------------------------------------- scheduler

#[test]
fn interval_defaults_to_two_hours_and_is_configurable() {
    let conn = open_in_memory().unwrap();
    assert_eq!(interval(&conn), Duration::from_secs(7200));
    crate::db::settings::set(&conn, crate::db::settings::SPOTIFY_SYNC_INTERVAL, "900").unwrap();
    assert_eq!(interval(&conn), Duration::from_secs(900));
    crate::db::settings::set(&conn, crate::db::settings::SPOTIFY_SYNC_INTERVAL, "5").unwrap();
    assert_eq!(interval(&conn), Duration::from_secs(MIN_INTERVAL_SECS), "clamped");
}

#[test]
fn engine_debounces_and_prevents_duplicate_runs() {
    let eng = SyncEngine::default();
    eng.request("p", Duration::from_secs(0));
    eng.request("p", Duration::from_secs(0)); // coalesced
    eng.request("q", Duration::from_secs(60));
    let now = Instant::now() + Duration::from_millis(5);
    assert_eq!(eng.take_due(now), vec!["p".to_string()]);
    assert!(eng.take_due(now).is_empty());
    assert!(eng.next_due().is_some());

    let g = eng.begin("p").unwrap();
    assert!(eng.begin("p").is_none(), "second simultaneous sync is refused");
    assert!(eng.is_syncing("p"));
    drop(g);
    assert!(eng.begin("p").is_some());

    eng.back_off(Duration::from_secs(30));
    assert!(eng.backoff_until().is_some());
    assert!(!eng.is_shutdown());
    eng.shutdown();
    assert!(eng.is_shutdown());
}

// ------------------------------------- fork reconciled with Spotify

#[tokio::test]
async fn fork_uses_spotifys_latest_contents_and_unsynced_local_edits() {
    let e = env_with(owned("a b c")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    // Since the last sync: Spotify gained "d" after "a"; Branflakes gained "e".
    {
        let mut s = e.fake.lock().unwrap();
        s.items.insert(1, "spotify:track:d".into());
        s.snapshot += 1;
    }
    let ex = e.spotify_track("e");
    add_tracks(&e.conn(), &pid, &[ex], None, None).unwrap();
    let writes = e.writes();
    let local_before = e.local_uris(&pid);

    // User inserts a local file after "b" (position 2 in what they saw: a b c e).
    let l = e.other_track(Source::Local, "/song.mp3");
    let out = e.add(&pid, &[l.clone()], Some(2)).await;
    let AddOutcome::Forked { playlist: fork, from_latest: true, .. } = out else { panic!("{out:?}") };
    let got: Vec<String> = playlists::tracks(&e.conn(), &fork.id)
        .unwrap()
        .into_iter()
        .map(|t| if t.source == Source::Spotify { t.provider_id } else { "LOCAL".into() })
        .collect();
    assert_eq!(got, ["a", "d", "b", "LOCAL", "c", "e"], "Spotify's new track, local edit, and the intended position");

    // Nothing written anywhere for the original.
    assert_eq!(e.writes(), writes);
    assert_eq!(e.remote(), uris("a d b c"));
    assert_eq!(e.local_uris(&pid), local_before);
}

#[tokio::test]
async fn fork_falls_back_to_last_sync_when_spotify_is_unreachable_and_says_so() {
    let e = env_with(owned("a b")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        format!("http://127.0.0.1:{port}")
    };
    let offline = SpotifyAdapter::with_base(http::client(), e.store.clone(), &format!("{dead}/v1"), &dead, RetryPolicy::immediate());
    let ctx = SyncCtx { db: &e.db, spotify: &offline, client_id: "cid".into(), artwork_dir: e.art.path().into() };
    let l = e.other_track(Source::Local, "/x.mp3");
    let out = add_tracks_reconciled(&e.db, Ok(&ctx), None, &pid, &[l], None, Duration::from_secs(5)).await.unwrap();
    let AddOutcome::Forked { playlist, from_latest: false, note, .. } = out else { panic!("{out:?}") };
    assert!(note.contains("couldn't be checked") && note.contains("since then are not included"), "{note}");
    assert_eq!(playlists::get(&e.conn(), &playlist.id).unwrap().fork_note.as_deref(), Some(note.as_str()));
    assert_eq!(playlists::track_ids(&e.conn(), &playlist.id).unwrap().len(), 3);

    // Not connected at all: same fallback, clear reason.
    let l2 = e.other_track(Source::Local, "/y.mp3");
    let out = add_tracks_reconciled(&e.db, Err("Spotify isn't connected".into()), None, &pid, &[l2], None, Duration::from_secs(5)).await.unwrap();
    let AddOutcome::Forked { note, from_latest: false, .. } = out else { panic!("{out:?}") };
    assert!(note.contains("Spotify isn't connected"), "{note}");
}

#[tokio::test]
async fn fork_times_out_to_the_last_sync() {
    let e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    // A sync holds the run lock for longer than the fork is willing to wait.
    let lock = tokio::sync::Mutex::new(());
    let _held = lock.lock().await;
    let l = e.other_track(Source::Local, "/z.mp3");
    let out = add_tracks_reconciled(&e.db, Ok(&e.ctx()), Some(&lock), &pid, &[l], None, Duration::from_millis(50)).await.unwrap();
    let AddOutcome::Forked { from_latest: false, note, .. } = out else { panic!("{out:?}") };
    assert!(note.contains("didn't respond in time"), "{note}");
}

#[tokio::test]
async fn fork_reports_spotify_entries_it_cannot_copy() {
    let mut st = owned("a spotify:local:Artist:Album:Home+demo:120 b");
    st.items.push("spotify:episode:ep1".into());
    st.items.push("spotify:unavailable:x".into()); // served as {"item": null}
    let e = env_with(st).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let l = e.other_track(Source::Soundcloud, "soundcloud:tracks:1");
    let out = e.add(&pid, &[l], None).await;
    let AddOutcome::Forked { playlist, uncopyable, note, .. } = out else { panic!("{out:?}") };
    let kinds: Vec<&str> = uncopyable.iter().map(|u| u.kind.as_str()).collect();
    assert_eq!(kinds, ["spotify_local_file", "episode", "unavailable"], "{uncopyable:?}");
    assert_eq!(uncopyable[0].name.as_deref(), Some("Local"));
    assert_eq!(uncopyable[1].name.as_deref(), Some("Ep 1"));
    assert!(note.contains("Not copied: 3") && note.contains("not complete"), "{note}");
    assert!(note.contains("1 Spotify local file") && note.contains("1 podcast episode") && note.contains("1 unavailable Spotify entry"), "{note}");
    // a, b and the new track only.
    assert_eq!(playlists::track_ids(&e.conn(), &playlist.id).unwrap().len(), 3);
}

#[test]
fn fork_note_wording() {
    let src = ForkSource {
        track_ids: vec![],
        uncopyable: vec![UncopyableEntry { kind: "episode".into(), name: Some("Ep 1".into()) }],
        latest: true,
        fallback_reason: None,
        last_synced_at: None,
    };
    let n = fork_note("Mix", &src);
    assert!(n.contains("Copied from “Mix”") && n.contains("1 podcast episode") && n.contains("“Ep 1”"), "{n}");
}

// --------------------------------------------- cover change detection

#[test]
fn cover_identity_ignores_host_size_and_query() {
    assert_eq!(
        image_key("https://image-cdn-ak.spotifycdn.com/image/ab67706c0000da84abc?x=1"),
        image_key("https://image-cdn-fa.spotifycdn.com/image/ab67706c0000da84abc")
    );
    assert_eq!(image_key("https://mosaic.scdn.co/640/aaa/bbb"), image_key("https://mosaic.scdn.co/300/aaa/bbb"));
    assert_ne!(image_key("https://mosaic.scdn.co/640/aaa/bbb"), image_key("https://mosaic.scdn.co/640/aaa/ccc"));
    assert_ne!(image_key("https://i.scdn.co/image/one"), image_key("https://i.scdn.co/image/two"));
}

#[tokio::test]
async fn same_url_new_image_is_caught_by_manual_sync_only() {
    let e = env_with(FakeState { image: Some("cover1".into()), ..owned("a") }).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let before = playlists::get(&e.conn(), &pid).unwrap().artwork_path.unwrap();
    let downloads = e.fake.lock().unwrap().image_downloads;

    // Spotify swaps the image behind the same URL.
    let mut other = Vec::new();
    image::RgbImage::from_pixel(4, 4, image::Rgb([0, 120, 255])).write_to(&mut std::io::Cursor::new(&mut other), image::ImageFormat::Png).unwrap();
    e.fake.lock().unwrap().image_bytes = Some(other);

    // Automatic sync: no download, cover unchanged (documented limitation).
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(e.fake.lock().unwrap().image_downloads, downloads);
    assert_eq!(playlists::get(&e.conn(), &pid).unwrap().artwork_path.as_deref(), Some(before.as_str()));

    // Manual sync: content check finds the new image.
    let r = sync_playlist(&e.ctx(), &pid, true).await.unwrap();
    assert!(r.artwork_changed);
    let after = playlists::get(&e.conn(), &pid).unwrap().artwork_path.unwrap();
    assert_ne!(after, before);

    // Another manual sync with identical bytes changes nothing.
    let r = sync_playlist(&e.ctx(), &pid, true).await.unwrap();
    assert!(!r.artwork_changed);
    assert_eq!(playlists::get(&e.conn(), &pid).unwrap().artwork_path.unwrap(), after);
}

// =================================================== 0.2.1 lifecycle tests

fn summary(owner: &str, collaborative: bool) -> crate::providers::spotify::SpotifyPlaylistSummary {
    crate::providers::spotify::SpotifyPlaylistSummary { id: "p1".into(), name: "Road trip".into(), owner_id: Some(owner.into()), total: None, collaborative }
}

fn link_count(e: &Env) -> i64 {
    e.conn().query_row("SELECT COUNT(*) FROM spotify_playlist_links", [], |r| r.get(0)).unwrap()
}

fn backdate_missing(e: &Env, pid: &str) {
    e.conn()
        .execute("UPDATE spotify_playlist_links SET missing_since = ?2 WHERE playlist_id = ?1", rusqlite::params![pid, (chrono::Utc::now() - chrono::Duration::minutes(2)).to_rfc3339()])
        .unwrap();
}

fn offline_adapter(e: &Env) -> SpotifyAdapter {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    let dead = format!("http://127.0.0.1:{port}");
    SpotifyAdapter::with_base(http::client(), e.store.clone(), &format!("{dead}/v1"), &dead, RetryPolicy::immediate())
}

// ---------------------------------------------------------- import

#[tokio::test]
async fn import_enables_sync_and_reimport_never_duplicates() {
    let e = env_with(owned("a b")).await;
    let r = import_playlist(&e.ctx(), &summary("me", false), "me").await.unwrap();
    let ImportResult::Synced { playlist_id: pid } = r else { panic!("{r:?}") };
    let p = playlists::get(&e.conn(), &pid).unwrap();
    assert!(p.sync.is_some(), "Spotify Synced right after import");
    assert_eq!(e.local_uris(&pid), uris("a b"));

    // Unsynced Branflakes edit, then import again: no duplicate playlist or
    // link, and the edit is not overwritten.
    let c = e.spotify_track("c");
    add_tracks(&e.conn(), &pid, &[c], None, None).unwrap();
    let r = import_playlist(&e.ctx(), &summary("me", false), "me").await.unwrap();
    assert_eq!(r, ImportResult::AlreadySynced { playlist_id: pid.clone() });
    assert_eq!(playlists::list(&e.conn()).unwrap().len(), 1);
    assert_eq!(link_count(&e), 1);
    assert_eq!(e.local_uris(&pid), uris("a b c"));
    // The next sync then pushes the edit.
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(e.remote(), uris("a b c"));
}

#[tokio::test]
async fn import_of_ineligible_playlists_is_plain_with_a_reason() {
    // Someone else's (not collaborative) playlist.
    let e = env_with(FakeState { owner: "other".into(), ..owned("a") }).await;
    let r = import_playlist(&e.ctx(), &summary("other", false), "me").await.unwrap();
    let ImportResult::Plain { playlist_id: Some(pid), reason } = r else { panic!("{r:?}") };
    assert!(reason.contains("belongs to another Spotify user"), "{reason}");
    assert!(playlists::get(&e.conn(), &pid).unwrap().sync.is_none());
    assert_eq!(link_count(&e), 0);

    // Missing playlist-write permissions.
    let e = env_with(owned("a")).await;
    e.spotify.disconnect().await.unwrap();
    e.store.set(SPOTIFY_TOKENS, &tokens("playlist-read-private user-library-read", 3600)).unwrap();
    let r = import_playlist(&e.ctx(), &summary("me", false), "me").await.unwrap();
    let ImportResult::Plain { reason, .. } = r else { panic!("{r:?}") };
    assert!(reason.contains("Reconnect Spotify"), "{reason}");
    assert_eq!(link_count(&e), 0);
}

#[tokio::test]
async fn import_never_overwrites_a_copy_with_non_spotify_tracks() {
    let e = env_with(owned("a b")).await;
    let pid = playlists::create(&e.conn(), "Road trip", Some("spotify:playlist:p1")).unwrap().id;
    let a = e.spotify_track("a");
    let l = e.other_track(Source::Local, "/mine.mp3");
    playlists::set_items(&e.conn(), &pid, &[a.clone(), l.clone()]).unwrap();
    let r = import_playlist(&e.ctx(), &summary("me", false), "me").await.unwrap();
    let ImportResult::Plain { reason, .. } = r else { panic!("{r:?}") };
    assert!(reason.contains("left unchanged"), "{reason}");
    assert_eq!(playlists::track_ids(&e.conn(), &pid).unwrap(), vec![a, l]);
}

// --------------------------------------------- create and link to Spotify

fn new_local_playlist(e: &Env, name: &str, spotify_ids: &[&str]) -> String {
    let ids: Vec<String> = spotify_ids.iter().map(|i| e.spotify_track(i)).collect();
    let pid = playlists::create(&e.conn(), name, None).unwrap().id;
    playlists::set_items(&e.conn(), &pid, &ids).unwrap();
    pid
}

#[tokio::test]
async fn linking_creates_a_private_spotify_playlist_and_syncs_contents() {
    let e = env_with(FakeState { gone: true, ..Default::default() }).await;
    let pid = new_local_playlist(&e, "New mix", &["a", "b"]);
    assert_eq!(create_and_link(&e.ctx(), &pid).await.unwrap(), "p1");
    assert_eq!(e.fake.lock().unwrap().creates, 1);
    assert!(playlists::get(&e.conn(), &pid).unwrap().sync.is_some());
    sync_playlist(&e.ctx(), &pid, true).await.unwrap();
    assert_eq!(e.remote(), uris("a b"));
    assert_eq!(e.fake.lock().unwrap().name, "New mix");
    // Linking again is a no-op; later syncs change nothing.
    create_and_link(&e.ctx(), &pid).await.unwrap();
    let w = e.writes();
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    let creates = e.fake.lock().unwrap().creates;
    assert_eq!((creates, e.writes()), (1, w));
}

#[tokio::test]
async fn retry_after_an_unknown_create_outcome_adopts_instead_of_duplicating() {
    let e = env_with(FakeState { gone: true, create_then_fail: 1, ..Default::default() }).await;
    let pid = new_local_playlist(&e, "New mix", &["a"]);
    assert!(create_and_link(&e.ctx(), &pid).await.is_err(), "the answer was lost");
    assert_eq!(e.fake.lock().unwrap().creates, 1, "POST is not blindly retried");
    assert!(links::pending_create(&e.conn(), &pid).unwrap().is_some());
    assert!(playlists::get(&e.conn(), &pid).unwrap().sync.is_none(), "local playlist kept, not linked yet");
    // Retry: the playlist Spotify did create is adopted.
    assert_eq!(create_and_link(&e.ctx(), &pid).await.unwrap(), "p1");
    assert_eq!(e.fake.lock().unwrap().creates, 1, "no duplicate Spotify playlist");
    assert!(links::pending_create(&e.conn(), &pid).unwrap().is_none());
}

#[tokio::test]
async fn failed_or_impossible_creation_keeps_the_local_playlist() {
    // Refused by Spotify: definitely nothing created, marker cleared.
    let e = env_with(FakeState { gone: true, create_refused: true, ..Default::default() }).await;
    let pid = new_local_playlist(&e, "Mine", &["a"]);
    assert!(create_and_link(&e.ctx(), &pid).await.is_err());
    assert!(links::pending_create(&e.conn(), &pid).unwrap().is_none());
    assert_eq!(playlists::track_ids(&e.conn(), &pid).unwrap().len(), 1);

    // Non-Spotify tracks: explained, nothing created.
    let e = env_with(FakeState { gone: true, ..Default::default() }).await;
    let pid = new_local_playlist(&e, "Mixed", &["a"]);
    let l = e.other_track(Source::Local, "/x.mp3");
    playlists::add_tracks(&e.conn(), &pid, &[l]).unwrap();
    let err = create_and_link(&e.ctx(), &pid).await.unwrap_err();
    assert!(err.to_string().contains("aren't on Spotify"), "{err}");
    assert_eq!(e.fake.lock().unwrap().creates, 0);

    // Spotify disconnected: local playlist unaffected.
    let e = env_with(FakeState { gone: true, ..Default::default() }).await;
    let pid = new_local_playlist(&e, "Offline", &["a"]);
    e.spotify.disconnect().await.unwrap();
    assert!(create_and_link(&e.ctx(), &pid).await.is_err());
    assert!(playlists::get(&e.conn(), &pid).is_ok());
    assert_eq!(e.fake.lock().unwrap().creates, 0);
}

// ---------------------------------------------------------- deletion

#[tokio::test]
async fn deleting_an_owned_playlist_removes_it_from_the_library_once() {
    let e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let msg = delete_synced(&e.ctx(), &pid).await.unwrap();
    assert!(msg.contains("deleted from Spotify"), "{msg}");
    assert_eq!(e.fake.lock().unwrap().library_removals, 1);
    assert!(playlists::get(&e.conn(), &pid).is_err());
    assert_eq!(link_count(&e), 0);
}

#[tokio::test]
async fn deleting_someone_elses_playlist_only_unfollows_it() {
    let e = env_with(FakeState { owner: "other".into(), collaborative: true, ..owned("a") }).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let msg = delete_synced(&e.ctx(), &pid).await.unwrap();
    assert!(msg.contains("removed from your Spotify library") && msg.contains("owner's playlist is unchanged"), "{msg}");
    let s = e.fake.lock().unwrap();
    assert_eq!(s.library_removals, 1);
    assert!(!s.gone, "the owner's playlist still exists");
}

#[tokio::test]
async fn deletion_is_not_repeated_when_already_removed() {
    let e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    e.fake.lock().unwrap().not_in_library = true;
    delete_synced(&e.ctx(), &pid).await.unwrap();
    assert_eq!(e.fake.lock().unwrap().library_removals, 0);
    assert!(playlists::get(&e.conn(), &pid).is_err());
}

#[tokio::test]
async fn failed_deletion_keeps_the_playlist_and_link() {
    let e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    e.fake.lock().unwrap().remove_fails = true;
    assert!(delete_synced(&e.ctx(), &pid).await.is_err());
    assert!(playlists::get(&e.conn(), &pid).unwrap().sync.is_some());
    assert!(!links::is_pending_delete(&e.conn(), &pid).unwrap());
    // Offline: same.
    let off = offline_adapter(&e);
    let ctx = SyncCtx { db: &e.db, spotify: &off, client_id: "cid".into(), artwork_dir: e.art.path().into() };
    assert!(delete_synced(&ctx, &pid).await.is_err());
    assert!(playlists::get(&e.conn(), &pid).unwrap().sync.is_some());
    // Retry when it works.
    e.fake.lock().unwrap().remove_fails = false;
    delete_synced(&e.ctx(), &pid).await.unwrap();
    assert_eq!(e.fake.lock().unwrap().library_removals, 1);
}

#[tokio::test]
async fn interrupted_deletion_is_finished_or_undone_after_restart() {
    let e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    links::set_pending_delete(&e.conn(), &pid, true).unwrap();
    assert!(sync_playlist(&e.ctx(), &pid, false).await.is_err(), "no sync while deleting");
    // Spotify never got the removal: deletion undone, keeps syncing.
    assert!(!resume_delete(&e.ctx(), &pid).await.unwrap());
    assert!(playlists::get(&e.conn(), &pid).unwrap().sync.is_some());
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    // Spotify did remove it before the restart: finished locally.
    links::set_pending_delete(&e.conn(), &pid, true).unwrap();
    e.fake.lock().unwrap().not_in_library = true;
    assert!(resume_delete(&e.ctx(), &pid).await.unwrap());
    assert!(playlists::get(&e.conn(), &pid).is_err());
}

#[tokio::test]
async fn local_only_playlists_and_forks_never_touch_spotify_on_delete() {
    let e = env_with(owned("a b")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let l = e.other_track(Source::Local, "/f.mp3");
    let AddOutcome::Forked { playlist: fork, .. } = e.add(&pid, &[l], None).await else { panic!() };
    let plain = playlists::create(&e.conn(), "Plain", None).unwrap();
    // delete_synced refuses anything that isn't linked.
    assert!(matches!(delete_synced(&e.ctx(), &fork.id).await, Err(AppError::NotFound(_))));
    assert!(matches!(delete_synced(&e.ctx(), &plain.id).await, Err(AppError::NotFound(_))));
    playlists::delete(&e.conn(), &fork.id).unwrap();
    playlists::delete(&e.conn(), &plain.id).unwrap();
    let s = e.fake.lock().unwrap();
    assert_eq!((s.library_removals, s.writes), (0, 0));
    drop(s);
    assert!(playlists::get(&e.conn(), &pid).unwrap().sync.is_some(), "original untouched");
}

// ------------------------------------------- removed outside Branflakes

#[tokio::test]
async fn spotify_deletion_is_confirmed_by_a_second_check_before_removal() {
    let e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let plain = playlists::create(&e.conn(), "Unrelated", None).unwrap();
    e.fake.lock().unwrap().gone = true;
    let err = sync_playlist(&e.ctx(), &pid, false).await.unwrap_err();
    assert!(err.to_string().contains("no longer exists") && err.to_string().contains("check again"), "{err}");
    assert!(playlists::get(&e.conn(), &pid).is_ok(), "not removed on the first sighting");
    // Immediately again (e.g. a quick manual sync): still kept.
    assert!(sync_playlist(&e.ctx(), &pid, true).await.is_err());
    assert!(playlists::get(&e.conn(), &pid).is_ok());
    // A later, separate check confirms it.
    backdate_missing(&e, &pid);
    let r = sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert!(r.removed.unwrap().contains("no longer exists"));
    assert!(playlists::get(&e.conn(), &pid).is_err());
    assert_eq!(link_count(&e), 0);
    assert!(playlists::get(&e.conn(), &plain.id).is_ok(), "unrelated playlists untouched");
}

#[tokio::test]
async fn removal_from_the_library_is_detected_separately() {
    // Owned: Spotify's "delete" = removed from the library.
    let e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    e.fake.lock().unwrap().not_in_library = true;
    assert!(sync_playlist(&e.ctx(), &pid, false).await.is_err());
    backdate_missing(&e, &pid);
    let r = sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert!(r.removed.unwrap().contains("was deleted in Spotify"));

    // Someone else's playlist the user unfollowed.
    let e = env_with(FakeState { owner: "other".into(), collaborative: true, ..owned("a") }).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    e.fake.lock().unwrap().not_in_library = true;
    assert!(sync_playlist(&e.ctx(), &pid, false).await.is_err());
    backdate_missing(&e, &pid);
    let r = sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    let msg = r.removed.unwrap();
    assert!(msg.contains("removed from your Spotify library") && !msg.contains("deleted"), "{msg}");
}

#[tokio::test]
async fn a_playlist_that_comes_back_clears_the_missing_state() {
    let e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    e.fake.lock().unwrap().not_in_library = true;
    assert!(sync_playlist(&e.ctx(), &pid, false).await.is_err());
    e.fake.lock().unwrap().not_in_library = false; // re-followed / restored
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert!(links::missing_state(&e.conn(), &pid).unwrap().is_none());
    backdate_missing(&e, &pid);
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert!(playlists::get(&e.conn(), &pid).is_ok());
}

#[tokio::test]
async fn temporary_errors_never_count_as_deletion() {
    let mut e = env_with(owned("a")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    // Library check fails (5xx), twice with time in between.
    e.fake.lock().unwrap().contains_fails = true;
    for _ in 0..2 {
        assert!(sync_playlist(&e.ctx(), &pid, false).await.is_err());
        assert!(links::missing_state(&e.conn(), &pid).unwrap().is_none());
        backdate_missing(&e, &pid);
    }
    e.fake.lock().unwrap().contains_fails = false;
    // Rate limited.
    e.fake.lock().unwrap().throttle_meta = 10;
    assert!(matches!(sync_playlist(&e.ctx(), &pid, false).await, Err(AppError::RateLimited { .. })));
    e.fake.lock().unwrap().throttle_meta = 0;
    // Offline.
    let off = offline_adapter(&e);
    let ctx = SyncCtx { db: &e.db, spotify: &off, client_id: "cid".into(), artwork_dir: e.art.path().into() };
    assert!(matches!(sync_playlist(&ctx, &pid, false).await, Err(AppError::Network { .. })));
    // Expired sign-in.
    e.spotify.disconnect().await.unwrap();
    e.store.set(SPOTIFY_TOKENS, &tokens(&SCOPES.join(" "), -100)).unwrap();
    e.server.mock("POST", "/api/token").with_status(400).with_body(r#"{"error":"invalid_grant"}"#).create_async().await;
    assert!(matches!(sync_playlist(&e.ctx(), &pid, false).await, Err(AppError::Auth { .. })));
    // Nothing was ever recorded as missing, nothing removed, tracks still available.
    assert!(links::missing_state(&e.conn(), &pid).unwrap().is_none());
    let tracks = playlists::tracks(&e.conn(), &pid).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].availability, crate::model::Availability::Available, "errors never mark tracks unavailable");
}

// ------------------------------------------------- alternatives + sync

#[tokio::test]
async fn alternatives_live_only_in_branflakes_and_survive_syncs() {
    use crate::db::alternatives::{self as alts, AddAlternativeOutcome};
    let e = env_with(owned("a b c")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let ids = playlists::track_ids(&e.conn(), &pid).unwrap();
    let b = library::get_track(&e.conn(), &ids[1]).unwrap();
    let alt_id = e.other_track(Source::Soundcloud, "soundcloud:tracks:9");
    let alt = library::get_track(&e.conn(), &alt_id).unwrap();
    assert_eq!(alts::add(&e.conn(), &pid, &b, &alt, Some(1)).unwrap(), AddAlternativeOutcome::Added { position: 2 });
    let writes = e.writes();

    // Not a Spotify change: nothing pushed, original kept on both sides.
    let r = sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(r.pushed_ops, 0);
    assert_eq!(e.writes(), writes);
    assert_eq!(e.remote(), uris("a b c"));
    let shape = |e: &Env| -> Vec<String> {
        playlists::tracks(&e.conn(), &pid).unwrap().into_iter().map(|t| if t.source == Source::Spotify { t.provider_id } else { "ALT".into() }).collect()
    };
    assert_eq!(shape(&e), ["a", "b", "ALT", "c"]);

    // Spotify changes around it: the alternative stays after its original.
    {
        let mut s = e.fake.lock().unwrap();
        s.items.insert(0, "spotify:track:d".into());
        s.snapshot += 1;
    }
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(shape(&e), ["d", "a", "b", "ALT", "c"]);
    // Original removed on Spotify: the alternative is kept (where it was).
    {
        let mut s = e.fake.lock().unwrap();
        s.items.retain(|u| u != "spotify:track:b");
        s.snapshot += 1;
    }
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(shape(&e), ["d", "a", "ALT", "c"]);

    // User removes it: never re-added by later syncs.
    let pos = playlists::track_ids(&e.conn(), &pid).unwrap().iter().position(|x| *x == alt_id).unwrap();
    playlists::remove_at(&e.conn(), &pid, pos).unwrap();
    alts::mark_removed_if_gone(&e.conn(), &pid, &alt_id).unwrap();
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    sync_playlist(&e.ctx(), &pid, false).await.unwrap();
    assert_eq!(shape(&e), ["d", "a", "c"]);
    assert!(alts::list(&e.conn(), &pid).unwrap()[0].removed, "the choice is remembered");
}

#[tokio::test]
async fn forks_keep_alternatives_next_to_their_originals() {
    use crate::db::alternatives as alts;
    let e = env_with(owned("a b")).await;
    let pid = enable(&e.ctx(), "p1").await.unwrap();
    let ids = playlists::track_ids(&e.conn(), &pid).unwrap();
    let a = library::get_track(&e.conn(), &ids[0]).unwrap();
    let alt_id = e.other_track(Source::Local, "/alt.mp3");
    let alt = library::get_track(&e.conn(), &alt_id).unwrap();
    alts::add(&e.conn(), &pid, &a, &alt, Some(0)).unwrap();
    let l = e.other_track(Source::Local, "/new.mp3");
    let AddOutcome::Forked { playlist: fork, .. } = e.add(&pid, &[l.clone()], None).await else { panic!() };
    assert_eq!(playlists::track_ids(&e.conn(), &fork.id).unwrap(), vec![ids[0].clone(), alt.id.clone(), ids[1].clone(), l]);
}
