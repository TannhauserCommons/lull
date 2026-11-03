//! Windows' media overlay and media keys for Classic's own player: the System Media Transport
//! Controls of the Classic window (the web UI gets them from WebView2's Media Session; with no
//! webview, the window asks Windows itself). The overlay shows the song, its album and cover, and
//! whether it plays; its buttons and the keyboard's media keys come back as the tray's actions.
#![allow(unsafe_code)]

use super::hub::Hub;
use crate::desktop::state::NowPlaying;
use std::io::Read;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use windows::Foundation::TypedEventHandler;
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls, SystemMediaTransportControlsButton,
    SystemMediaTransportControlsButtonPressedEventArgs,
};
use windows::Storage::Streams::{IRandomAccessStream, RandomAccessStreamReference};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::{
    BSOS_DEFAULT, CreateRandomAccessStreamOverStream, ISystemMediaTransportControlsInterop,
};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::core::{HSTRING, Ref};

/// No cover is bigger than this.
const MAX_ART: u64 = 8 * 1024 * 1024;

pub struct Overlay {
    controls: SystemMediaTransportControls,
    token: i64,
    /// The song the overlay shows, so a cover fetched late doesn't land on the next one.
    song: Arc<Mutex<String>>,
}

impl Overlay {
    /// The window's controls, their buttons on, and what they do.
    pub fn new(hwnd: HWND, app: AppHandle) -> windows::core::Result<Self> {
        let interop = windows::core::factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
        // SAFETY: a live window of this thread.
        let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(hwnd)? };
        controls.SetIsEnabled(true)?;
        controls.SetIsPlayEnabled(true)?;
        controls.SetIsPauseEnabled(true)?;
        controls.SetIsStopEnabled(true)?;
        controls.SetIsNextEnabled(true)?;
        controls.SetIsPreviousEnabled(true)?;
        let pressed = TypedEventHandler::new(
            move |_, args: Ref<SystemMediaTransportControlsButtonPressedEventArgs>| {
                let action = match args.ok()?.Button()? {
                    SystemMediaTransportControlsButton::Play => "play",
                    SystemMediaTransportControlsButton::Pause => "pause",
                    SystemMediaTransportControlsButton::Stop => "stop",
                    SystemMediaTransportControlsButton::Next => "next",
                    SystemMediaTransportControlsButton::Previous => "previous",
                    _ => return Ok(()),
                };
                if let Err(e) = super::act(&app, action) {
                    eprintln!("lull: media key {action}: {e}");
                }
                Ok(())
            },
        );
        let token = controls.ButtonPressed(&pressed)?;
        Ok(Self {
            controls,
            token,
            song: Arc::default(),
        })
    }

    /// What the overlay shows: the song, and whether it plays. A new song's cover comes from the
    /// hub on a thread of its own.
    pub fn show(&self, now: &NowPlaying, art: Option<&str>, hub: &Hub) -> windows::core::Result<()> {
        self.controls.SetPlaybackStatus(if now.playing {
            MediaPlaybackStatus::Playing
        } else {
            MediaPlaybackStatus::Paused
        })?;
        {
            let mut song = self.song.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if *song == now.id {
                return Ok(());
            }
            song.clone_from(&now.id);
        }
        let updater = self.controls.DisplayUpdater()?;
        updater.ClearAll()?;
        updater.SetType(MediaPlaybackType::Music)?;
        let music = updater.MusicProperties()?;
        music.SetTitle(&HSTRING::from(now.title.as_str()))?;
        music.SetArtist(&HSTRING::from(now.artist.as_str()))?;
        music.SetAlbumTitle(&HSTRING::from(now.album.as_str()))?;
        updater.Update()?;
        let Some(art) = art else {
            return Ok(());
        };
        let (controls, song, id, url, hub) = (
            self.controls.clone(),
            self.song.clone(),
            now.id.clone(),
            art.to_owned(),
            hub.clone(),
        );
        let spawned = std::thread::Builder::new().name("classic-art".into()).spawn(move || {
            let shown = cover(&hub, &url).and_then(|stream| {
                if *song.lock().unwrap_or_else(std::sync::PoisonError::into_inner) != id {
                    return Ok(());
                }
                let updater = controls.DisplayUpdater()?;
                updater.SetThumbnail(&stream)?;
                updater.Update()
            });
            if let Err(e) = shown {
                eprintln!("lull: no cover in the media overlay: {e}");
            }
        });
        if let Err(e) = spawned {
            eprintln!("lull: no cover in the media overlay: {e}");
        }
        Ok(())
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        let _ = self.controls.RemoveButtonPressed(self.token);
        let _ = self.controls.SetIsEnabled(false);
    }
}

/// The cover from the hub, as a stream the overlay takes.
fn cover(hub: &Hub, url: &str) -> windows::core::Result<RandomAccessStreamReference> {
    let mut bytes = Vec::new();
    hub.get(url)
        .and_then(|reader| reader.take(MAX_ART).read_to_end(&mut bytes).map_err(|e| e.to_string()))
        .map_err(|e| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e))?;
    // SAFETY: a memory stream holding a copy of the bytes, wrapped as a WinRT stream.
    let stream: IRandomAccessStream = unsafe {
        let memory = SHCreateMemStream(Some(&bytes)).ok_or_else(windows::core::Error::from_thread)?;
        CreateRandomAccessStreamOverStream(&memory, BSOS_DEFAULT)?
    };
    RandomAccessStreamReference::CreateFromStream(&stream)
}
