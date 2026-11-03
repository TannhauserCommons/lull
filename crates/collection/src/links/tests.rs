use std::sync::atomic::Ordering;

use super::*;
use crate::testing::{published, service_with_fakes};

fn link() -> Result<Link> {
    Link::from_key("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M")
}

fn song(key: &str, artist: &str, title: &str) -> Song {
    Song {
        key: key.into(),
        artist: artist.into(),
        title: title.into(),
        album: None,
        duration: Some(200),
        isrc: None,
        added: None,
        picture: None,
    }
}

fn listing(songs: Vec<Song>) -> Listing {
    Listing {
        name: "Road Trip".into(),
        songs,
        note: None,
        covers: Vec::new(),
        description: Some("Songs for the long drive.".into()),
        owner: None,
    }
}

/// Runs an import of `listing` as the job would after reading the link.
async fn import_now(svc: &Arc<Service>, listing: Listing) -> Result<Value> {
    import_saving(svc, listing, None).await
}

/// `import_now` with the request's "Save the songs too" (`None`: not said).
async fn import_saving(svc: &Arc<Service>, listing: Listing, save: Option<bool>) -> Result<Value> {
    let link = link()?;
    let key = link.key();
    let seq = svc.write(move |tx, _| begin(tx, &key, "Road Trip", save)).await?;
    let mut job = Job::start(svc, seq, "Road Trip".into(), lull_core::now());
    let result = import(svc, &link, Some(listing), &mut job).await;
    job.finish(result);
    Ok(one(State(svc.clone()), Path(format!("lk-{seq}"))).await?.0)
}

async fn choose_now(svc: &Arc<Service>, pos: usize, choice: Value) -> Result<Value> {
    let choice: Choice = serde_json::from_value(choice).map_err(|e| Error::internal(e.to_string()))?;
    Ok(choose(State(svc.clone()), Path(("lk-1".into(), pos)), Ok(Json(choice)))
        .await?
        .0)
}

async fn playlist_ids(svc: &Service, id: &str) -> Result<Vec<String>> {
    let seq = playlists::seq_of(id)?;
    let p = svc.ctx.db.call(move |c| playlists::one(c, seq)).await?;
    Ok(p.map(|p| p.track_ids.into_iter().map(|t| t.0).collect())
        .unwrap_or_default())
}

async fn known(svc: &Service, fake: &crate::testing::FakeCatalog, artist: &str, title: &str) -> Result<String> {
    let id = crate::testing::known_song(svc, artist, title).await?;
    let track = svc.ctx.db.call({
        let id = id.clone();
        move |c| catalog::track(c, &id)
    });
    if let Some(t) = track.await? {
        fake.songs.lock().map_err(|_| Error::internal("poisoned"))?.push(t);
    }
    Ok(id.0)
}

#[tokio::test]
async fn a_link_becomes_a_playlist_of_its_matched_songs_and_lists_the_rest() -> Result<()> {
    let (svc, mut rx, _, fake) = service_with_fakes().await?;
    let (a, b) = (
        known(&svc, &fake, "Velvet Orchard", "Paper Moon").await?,
        known(&svc, &fake, "Low Tide Radio", "Hello").await?,
    );
    let out = import_now(
        &svc,
        listing(vec![
            song("spotify:track:1", "Velvet Orchard", "Paper Moon"),
            song("spotify:track:2", "Nobody Knows", "Lost Song"),
            song("spotify:track:3", "Low Tide Radio", "Hello - 2011 Remaster"),
        ]),
    )
    .await?;

    assert_eq!(
        out["counts"],
        json!({ "matched": 2, "fixed": 0, "skipped": 0, "unmatched": 1 })
    );
    assert_eq!(out["songs"][1]["status"], "unmatched");
    let playlist = out["playlistId"].as_str().unwrap_or_default().to_owned();
    assert_eq!(playlist_ids(&svc, &playlist).await?, [a, b]);
    let done = published(&mut rx)
        .into_iter()
        .rev()
        .find(|e| e.topic == "job")
        .map(|e| e.data)
        .unwrap_or_default();
    assert_eq!(
        (
            done["state"].as_str(),
            done["matched"].as_u64(),
            done["unmatched"].as_u64()
        ),
        (Some("done"), Some(2), Some(1))
    );
    assert_eq!(done["playlistId"], json!(playlist));
    let made = svc.ctx.db.call(|c| playlists::all(c)).await?;
    assert_eq!(
        made[0].description, "Songs for the long drive.",
        "the source's description"
    );
    Ok(())
}

#[tokio::test]
async fn fixes_and_skips_change_the_playlist_at_once() -> Result<()> {
    let (svc, _rx, _, fake) = service_with_fakes().await?;
    let a = known(&svc, &fake, "Velvet Orchard", "Paper Moon").await?;
    let picked = crate::testing::known_song(&svc, "Somebody Else", "Lost Song (Live)").await?;
    let out = import_now(
        &svc,
        listing(vec![
            song("spotify:track:1", "Velvet Orchard", "Paper Moon"),
            song("spotify:track:2", "Nobody Knows", "Lost Song"),
            song("spotify:track:3", "Obscure Band", "Demo"),
        ]),
    )
    .await?;
    let playlist = out["playlistId"].as_str().unwrap_or_default().to_owned();

    choose_now(&svc, 1, json!({ "choice": "track", "trackId": picked.0 })).await?;
    let out = choose_now(&svc, 2, json!({ "choice": "as-is" })).await?;
    assert_eq!(
        out["counts"],
        json!({ "matched": 1, "fixed": 2, "skipped": 0, "unmatched": 0 })
    );
    let ids = playlist_ids(&svc, &playlist).await?;
    assert_eq!(ids.len(), 3);
    assert_eq!((ids[0].as_str(), ids[1].as_str()), (a.as_str(), picked.0.as_str()));
    assert_eq!(
        out["entities"]["tracks"][&ids[2]]["artist"], "Obscure Band",
        "as-is is in the catalog"
    );

    let out = choose_now(&svc, 0, json!({ "choice": "skip" })).await?;
    assert_eq!(out["songs"][0]["status"], "skipped");
    assert_eq!(playlist_ids(&svc, &playlist).await?.len(), 2);

    let out = choose_now(&svc, 0, json!({ "choice": "none" })).await?;
    assert_eq!(
        out["songs"][0]["status"], "matched",
        "undoing a skip brings the match back"
    );
    assert!(
        choose_now(&svc, 1, json!({ "choice": "track", "trackId": "t-unknown" }))
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn importing_a_link_again_updates_its_playlist_keeps_choices_and_reuses_matches() -> Result<()> {
    let (svc, _rx, _, fake) = service_with_fakes().await?;
    let a = known(&svc, &fake, "Velvet Orchard", "Paper Moon").await?;
    let first = import_now(
        &svc,
        listing(vec![
            song("spotify:track:1", "Velvet Orchard", "Paper Moon"),
            song("spotify:track:2", "Nobody Knows", "Lost Song"),
        ]),
    )
    .await?;
    let playlist = first["playlistId"].as_str().unwrap_or_default().to_owned();
    choose_now(&svc, 1, json!({ "choice": "skip" })).await?;
    let lookups = fake.lookups.load(Ordering::Relaxed);

    let b = known(&svc, &fake, "Low Tide Radio", "Hello").await?;
    let again = import_now(
        &svc,
        listing(vec![
            song("spotify:track:3", "Low Tide Radio", "Hello"),
            song("spotify:track:2", "Nobody Knows", "Lost Song"),
            song("spotify:track:1", "Velvet Orchard", "Paper Moon"),
        ]),
    )
    .await?;

    assert_eq!(again["id"], first["id"]);
    assert_eq!(
        again["playlistId"],
        json!(playlist),
        "the same playlist, not a second one"
    );
    assert_eq!(
        playlist_ids(&svc, &playlist).await?,
        [b, a],
        "the link's new order; the skip holds"
    );
    assert_eq!(again["songs"][1]["status"], "skipped");
    assert_eq!(
        fake.lookups.load(Ordering::Relaxed) - lookups,
        1,
        "only the new song is looked up: a match, and a recent \"nothing matches\", are remembered"
    );
    let all = svc.ctx.db.call(|c| playlists::all(c)).await?;
    assert_eq!(all.len(), 1);
    Ok(())
}

#[tokio::test]
async fn a_deleted_playlist_is_made_again_by_the_next_import() -> Result<()> {
    let (svc, _rx, _, fake) = service_with_fakes().await?;
    known(&svc, &fake, "Velvet Orchard", "Paper Moon").await?;
    let songs = || listing(vec![song("spotify:track:1", "Velvet Orchard", "Paper Moon")]);
    let first = import_now(&svc, songs()).await?;
    let seq = playlists::seq_of(first["playlistId"].as_str().unwrap_or_default())?;
    svc.write(move |tx, events| playlists::delete(tx, events, seq)).await?;
    assert_eq!(svc.ctx.db.call(|c| lines(c, 1).map(|l| l.len())).await?, 1);

    let again = import_now(&svc, songs()).await?;
    assert!(again["playlistId"].is_string());
    assert_ne!(again["playlistId"], first["playlistId"]);
    Ok(())
}

#[tokio::test]
async fn the_same_link_imports_once_at_a_time() -> Result<()> {
    let (svc, _rx, _, _) = service_with_fakes().await?;
    let key = link()?.key();
    let held = Claim::new(&svc, key.clone())?;
    assert!(matches!(Claim::new(&svc, key.clone()), Err(Error::Conflict(_))));
    drop(held);
    assert!(Claim::new(&svc, key).is_ok());
    Ok(())
}

#[tokio::test]
async fn a_song_kept_as_named_takes_its_sources_picture_as_its_cover() -> Result<()> {
    let (svc, _rx, _, _) = service_with_fakes().await?;
    let mut lost = song("youtube:abcdefghijk", "Nobody", "Lost Song");
    lost.picture = Some("https://i.ytimg.com/vi/abcdefghijk/hqdefault.jpg".into());
    import_now(&svc, listing(vec![lost])).await?;
    let out = choose_now(&svc, 0, json!({ "choice": "as-is" })).await?;
    let id = TrackId(out["songs"][0]["trackId"].as_str().unwrap_or_default().to_owned());
    let has_cover = svc
        .ctx
        .db
        .call(move |c| {
            let t = catalog::track(c, &id)?.ok_or_else(|| Error::internal("no track"))?;
            catalog::is_bare(c, &t).map(|bare| !bare)
        })
        .await?;
    assert!(
        has_cover,
        "the upload's thumbnail is the song's cover until the catalog's is found"
    );
    Ok(())
}

/// The track ids of the `saves.request` events published so far.
fn save_requests(rx: &mut tokio::sync::broadcast::Receiver<lull_core::events::Event>) -> Vec<Vec<String>> {
    published(rx)
        .into_iter()
        .filter(|e| e.topic == "saves.request")
        .map(|e| {
            e.data["trackIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .collect()
}

#[tokio::test]
async fn save_the_songs_too_asks_library_and_is_kept_for_the_link() -> Result<()> {
    let (svc, mut rx, _, fake) = service_with_fakes().await?;
    let a = known(&svc, &fake, "Velvet Orchard", "Paper Moon").await?;
    let songs = || {
        listing(vec![
            song("spotify:track:1", "Velvet Orchard", "Paper Moon"),
            song("spotify:track:2", "Nobody Knows", "Lost Song"),
        ])
    };

    let out = import_saving(&svc, songs(), Some(true)).await?;
    assert_eq!(out["saveSongs"], true);
    assert_eq!(
        save_requests(&mut rx),
        [vec![a.clone()]],
        "the playlist's songs, not the unmatched one"
    );

    // A re-import that doesn't say keeps the link's choice; a song kept as named joins the saves.
    import_saving(&svc, songs(), None).await?;
    assert_eq!(save_requests(&mut rx), [vec![a.clone()]]);
    let kept = choose_now(&svc, 1, json!({ "choice": "as-is" })).await?;
    let kept_id = kept["songs"][1]["trackId"].as_str().unwrap_or_default().to_owned();
    assert_eq!(save_requests(&mut rx), [vec![kept_id]]);

    // Turned off: nothing is asked.
    import_saving(&svc, songs(), Some(false)).await?;
    assert!(save_requests(&mut rx).is_empty());
    Ok(())
}
