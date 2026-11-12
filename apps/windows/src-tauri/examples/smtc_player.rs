//! CCS/1 PURPOSE:WN-47 test player for check-pc-media-android.cjs: a real SMTC session ("PC proof song N", an orange 400 px
//! thumbnail, 200 s, Playing) under its own explicit AUMID `WindConnect.TestPlayer`, so the PC app's debug filter
//! (WINDCONNECT_TEST_MEDIA_APP) follows only it and never the person's players. Plays no sound.
//! IO:stdout lines `ready`, `event <play|pause|next|previous|seek ms>` with a monotonic ms stamp; stdin `no-next` turns NEXT off,
//! `quit` ends. Synthetic data only.
#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    use std::io::{BufRead, Write};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use windows::core::HSTRING;
    use windows::Foundation::{TimeSpan, TypedEventHandler};
    use windows::Media::Playback::MediaPlayer;
    use windows::Media::{
        MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControlsButton,
        SystemMediaTransportControlsTimelineProperties,
    };
    use windows::Storage::Streams::{
        DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference,
    };

    unsafe {
        windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(
            "WindConnect.TestPlayer",
        ))?;
    }
    let started = Instant::now();
    let log = move |line: String| {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{} {line}", started.elapsed().as_millis());
        let _ = out.flush();
    };
    let player = MediaPlayer::new()?;
    player.CommandManager()?.SetIsEnabled(false)?;
    let smtc = player.SystemMediaTransportControls()?;
    smtc.SetIsEnabled(true)?;
    smtc.SetIsPlayEnabled(true)?;
    smtc.SetIsPauseEnabled(true)?;
    smtc.SetIsNextEnabled(true)?;
    smtc.SetIsPreviousEnabled(true)?;

    // A 400 px orange PNG, so the PC re-encodes it to ≤ 256 px.
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 400, 400);
        encoder.set_color(png::ColorType::Rgb);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[240u8, 163, 94].repeat(400 * 400))
            .unwrap();
    }
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream)?;
    writer.WriteBytes(&png)?;
    writer.StoreAsync()?.get()?;
    writer.DetachStream()?;
    stream.Seek(0)?;
    let thumbnail = RandomAccessStreamReference::CreateFromStream(&stream)?;

    let track = Arc::new(Mutex::new(1u32));
    let position = Arc::new(Mutex::new(30_000u64));
    let show = {
        let smtc = smtc.clone();
        let thumbnail = thumbnail.clone();
        let (track, position) = (track.clone(), position.clone());
        move || -> windows::core::Result<()> {
            let updater = smtc.DisplayUpdater()?;
            updater.SetType(MediaPlaybackType::Music)?;
            let music = updater.MusicProperties()?;
            music.SetTitle(&HSTRING::from(format!(
                "PC proof song {}",
                track.lock().unwrap()
            )))?;
            music.SetArtist(&HSTRING::from("WindConnect PC test"))?;
            updater.SetThumbnail(&thumbnail)?;
            updater.Update()?;
            let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
            let ms = |v: u64| TimeSpan {
                Duration: v as i64 * 10_000,
            };
            timeline.SetStartTime(ms(0))?;
            timeline.SetEndTime(ms(200_000))?;
            timeline.SetMinSeekTime(ms(0))?;
            timeline.SetMaxSeekTime(ms(200_000))?;
            timeline.SetPosition(ms(*position.lock().unwrap()))?;
            smtc.UpdateTimelineProperties(&timeline)?;
            Ok(())
        }
    };
    show()?;
    smtc.SetPlaybackStatus(MediaPlaybackStatus::Playing)?;
    {
        let (smtc2, show, track, log) = (smtc.clone(), show.clone(), track.clone(), log);
        smtc.ButtonPressed(&TypedEventHandler::new(
            move |_,
                  args: windows::core::Ref<
                windows::Media::SystemMediaTransportControlsButtonPressedEventArgs,
            >| {
                let button = args.as_ref().unwrap().Button()?;
                match button {
                    SystemMediaTransportControlsButton::Play => {
                        smtc2.SetPlaybackStatus(MediaPlaybackStatus::Playing)?;
                        log("event play".into());
                    }
                    SystemMediaTransportControlsButton::Pause => {
                        smtc2.SetPlaybackStatus(MediaPlaybackStatus::Paused)?;
                        log("event pause".into());
                    }
                    SystemMediaTransportControlsButton::Next => {
                        *track.lock().unwrap() += 1;
                        show()?;
                        log("event next".into());
                    }
                    SystemMediaTransportControlsButton::Previous => {
                        let mut t = track.lock().unwrap();
                        *t = (*t).max(2) - 1;
                        drop(t);
                        show()?;
                        log("event previous".into());
                    }
                    _ => {}
                }
                Ok(())
            },
        ))?;
    }
    {
        let (show, position, log) = (show.clone(), position.clone(), log);
        smtc.PlaybackPositionChangeRequested(&TypedEventHandler::new(
            move |_,
                  args: windows::core::Ref<
                windows::Media::PlaybackPositionChangeRequestedEventArgs,
            >| {
                let at = (args.as_ref().unwrap().RequestedPlaybackPosition()?.Duration / 10_000)
                    .max(0) as u64;
                *position.lock().unwrap() = at;
                show()?;
                log(format!("event seek {at}"));
                Ok(())
            },
        ))?;
    }
    log("ready".into());
    for line in std::io::stdin().lock().lines() {
        match line.unwrap_or_default().trim() {
            "no-next" => {
                smtc.SetIsNextEnabled(false)?;
                log("next-disabled".into());
            }
            "quit" => break,
            _ => {}
        }
    }
    smtc.SetPlaybackStatus(MediaPlaybackStatus::Closed)?;
    std::thread::sleep(Duration::from_millis(200));
    drop(player);
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
