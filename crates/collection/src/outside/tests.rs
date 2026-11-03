use std::sync::atomic::Ordering;

use super::*;
use crate::testing::{published, service_with_fakes};

const KEY: &str = "deezer:playlist:2647620188";

fn song(key: &str, artist: &str, title: &str) -> Song {
    Song {
        key: key.into(),
        artist: artist.into(),
        title: title.into(),
        album: None,
        duration: Some(200),
        isrc: None,
        added: None,
    }
}

/// A listing as if read from Deezer a moment ago, so no request goes out.
async fn keep(svc: &Service, songs: Vec<Song>) -> Result<()> {
    let kept = Kept {
        name: "Person of Interest Complete Soundtrack OST".into(),
        owner: Some("Márcio Moreira".into()),
        description: None,
        note: None,
        songs,
        fetched_at: lull_core::now(),
        cover_at: None,
    };
    store(svc, KEY.into(), kept).await
}

fn query(link: &str) -> std::result::Result<Query<LinkQuery>, QueryRejection> {
    Ok(Query(LinkQuery {
        link: link.into(),
        size: None,
    }))
}

/// Waits for the background matching to send its last batch.
async fn matched(rx: &mut tokio::sync::broadcast::Receiver<lull_core::events::Event>) -> Vec<Value> {
    let mut events = Vec::new();
    for _ in 0..200 {
        events.extend(
            published(rx)
                .into_iter()
                .filter(|e| e.topic == "outside")
                .map(|e| e.data),
        );
        if events.last().is_some_and(|e| e["done"] == true) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    events
}

#[tokio::test]
async fn a_preview_plays_at_once_and_its_songs_are_matched_in_the_background() -> Result<()> {
    let (svc, mut rx, _, fake) = service_with_fakes().await?;
    let known = crate::testing::known_song(&svc, "Ramin Djawadi", "Main Theme").await?;
    if let Some(t) = svc
        .ctx
        .db
        .call({
            let id = known.clone();
            move |c| catalog::track(c, &id)
        })
        .await?
    {
        fake.songs.lock().map_err(|_| Error::internal("poisoned"))?.push(t);
    }
    keep(
        &svc,
        vec![
            song("deezer:track:1", "Ramin Djawadi", "Main Theme"),
            song("deezer:track:2", "Nobody", "Lost"),
        ],
    )
    .await?;

    let Json(first) = preview(State(svc.clone()), query(KEY)).await?;
    assert_eq!(first["name"], "Person of Interest Complete Soundtrack OST");
    assert_eq!(
        (first["total"].as_u64(), first["bookmarked"].as_bool()),
        (Some(2), Some(false))
    );
    for s in first["songs"].as_array().into_iter().flatten() {
        assert_eq!(s["status"], "pending");
        let id = s["trackId"].as_str().unwrap_or_default();
        assert!(
            first["entities"]["tracks"][id].is_object(),
            "every song plays at once: {id}"
        );
    }

    let events = matched(&mut rx).await;
    let songs: Vec<&Value> = events
        .iter()
        .flat_map(|e| e["songs"].as_array().into_iter().flatten())
        .collect();
    assert_eq!(songs.len(), 2);
    assert!(
        songs
            .iter()
            .any(|s| s["status"] == "matched" && s["trackId"] == known.as_str())
    );
    assert!(songs.iter().any(|s| s["status"] == "unmatched" && s["pos"] == 1));

    let lookups = fake.lookups.load(Ordering::Relaxed);
    let Json(again) = preview(
        State(svc.clone()),
        query("https://www.deezer.com/en/playlist/2647620188"),
    )
    .await?;
    assert_eq!(again["songs"][0]["status"], "matched");
    assert_eq!(again["songs"][1]["status"], "unmatched");
    assert_eq!(
        fake.lookups.load(Ordering::Relaxed),
        lookups,
        "remembered matches aren't looked up again"
    );
    Ok(())
}

#[tokio::test]
async fn a_bookmark_keeps_the_link_lists_it_and_can_be_removed() -> Result<()> {
    let (svc, mut rx, _, _) = service_with_fakes().await?;
    keep(&svc, vec![song("deezer:track:2", "Nobody", "Lost")]).await?;

    let Json(saved) = save(State(svc.clone()), query(KEY)).await?;
    assert_eq!(
        (saved["site"].as_str(), saved["total"].as_u64()),
        (Some("deezer"), Some(1))
    );
    let Json(listed) = list(State(svc.clone())).await?;
    assert_eq!(listed["items"][0]["id"], KEY);
    assert_eq!(listed["items"][0]["owner"], "Márcio Moreira");
    assert_eq!(listed["items"][0]["url"], "https://www.deezer.com/playlist/2647620188");
    let Json(shown) = preview(State(svc.clone()), query(KEY)).await?;
    assert_eq!(shown["bookmarked"], true);

    assert_eq!(unsave(State(svc.clone()), query(KEY)).await?, StatusCode::NO_CONTENT);
    let Json(listed) = list(State(svc.clone())).await?;
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(0));
    let events: Vec<Value> = published(&mut rx)
        .into_iter()
        .filter(|e| e.topic == "bookmark")
        .map(|e| e.data)
        .collect();
    assert_eq!(
        events,
        [
            json!({ "id": KEY, "saved": true }),
            json!({ "id": KEY, "saved": false })
        ]
    );
    Ok(())
}

#[tokio::test]
async fn an_old_listing_nobody_bookmarked_is_forgotten() -> Result<()> {
    let (svc, _rx, _, _) = service_with_fakes().await?;
    let old = Kept {
        name: "Old".into(),
        owner: None,
        description: None,
        note: None,
        songs: vec![song("deezer:track:9", "A", "B")],
        fetched_at: lull_core::now() - KEEP_FOR - 1,
        cover_at: None,
    };
    store(&svc, "deezer:playlist:1".into(), old.clone()).await?;
    store(&svc, "deezer:playlist:2".into(), old).await?;
    assert!(load(&svc, "deezer:playlist:1".into()).await?.is_none());
    Ok(())
}

#[test]
fn a_key_or_a_link_names_the_playlist_and_anything_else_is_refused() {
    assert_eq!(link_of(KEY).map(|l| l.key()).ok().as_deref(), Some(KEY));
    assert_eq!(
        link_of("https://www.youtube.com/playlist?list=PLrFSfgR7Y3I1yKrovCWPyGNlFR4eoEgCd")
            .map(|l| l.key())
            .ok()
            .as_deref(),
        Some("youtube:playlist:PLrFSfgR7Y3I1yKrovCWPyGNlFR4eoEgCd")
    );
    assert!(matches!(link_of("https://example.com/x"), Err(Error::BadRequest(_))));
}
