//! CCS/1 M:WN-55 the phone's SMS in the "Messages" window (sms/1, PROTOCOL.md §9.11).
//! FLOW:session attach -> watch (when the phone offers sms/1) -> `sms-changed` to the window -> it refreshes the threads
//! and the open thread with `since` = its newest message (incremental); a thread opens from this memory cache at once.
//! SEC:texts only in memory (dropped with another phone or unpairing), never written or logged; the page shows them as
//! text (no HTML), control and bidi characters removed here.
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{Emitter, Manager};
use wind_core::{
    identity::PublicKey,
    session::Session,
    sms::{self, AskError, Status},
};

/// One page of a thread: the first open and each "older" step.
pub const PAGE: u32 = 1000;
const THREADS: u32 = 300;
const WINDOW: &str = "sms";

#[derive(Default)]
pub struct Sms(Mutex<State>);

/// A thread's messages by (date, mms, id).
type Known = BTreeMap<(u64, bool, u64), sms::Message>;

#[derive(Default)]
struct State {
    session: Option<Session>,
    phone: Option<PublicKey>,
    threads: Vec<sms::Thread>,
    /// By thread: messages by (date, mms, id), and whether the oldest was reached.
    messages: HashMap<u64, (Known, bool)>,
    photos: HashMap<String, Option<String>>,
    watch: Option<tauri::async_runtime::JoinHandle<()>>,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct SmsStatus {
    pub connected: bool,
    /// The phone offers sms/1 (its setting on and READ_SMS granted).
    pub available: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct PersonView {
    address: String,
    name: Option<String>,
    photo: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ThreadView {
    id: u64,
    date_ms: u64,
    title: String,
    snippet: String,
    people: Vec<PersonView>,
    unread: u32,
    count: u32,
}

#[derive(Serialize, Clone, Debug)]
pub struct MessageView {
    key: String,
    date_ms: u64,
    /// "in", "sent", "pending", "failed".
    folder: &'static str,
    body: String,
    sender: Option<String>,
    attachments: u32,
    /// WN-56: the MMS's pictures (`sms_part` with the message key's id).
    parts: Vec<u64>,
    /// WN-56: a sent SMS's delivery report: "pending", "delivered" or "failed".
    delivery: Option<&'static str>,
}

/// WN-56: what a send is doing, pushed as `sms-send` (id from `sms_send`).
#[derive(Serialize, Clone, Debug)]
pub struct SendEvent {
    id: u64,
    /// "queued", "sent", "delivered", "failed", or "refused" with `message`.
    state: &'static str,
    message: Option<String>,
    done: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct SimView {
    id: u64,
    label: String,
    default: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ThreadMessages {
    /// Oldest first.
    messages: Vec<MessageView>,
    /// The oldest message is here.
    complete: bool,
    /// Answered from memory only (the phone is gone).
    cached: bool,
}

/// Plain text for the page: control (but newline) and bidi-override characters out.
pub fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !((c.is_control() && *c != '\n')
                || ('\u{202A}'..='\u{202E}').contains(c)
                || ('\u{2066}'..='\u{2069}').contains(c)
                || *c == '\u{200E}'
                || *c == '\u{200F}')
        })
        .collect()
}

fn title(t: &sms::Thread) -> String {
    let names: Vec<String> = t
        .people
        .iter()
        .map(|p| clean(p.name.as_deref().unwrap_or(&p.address)))
        .collect();
    if names.is_empty() {
        crate::i18n::tr("sms.unknown_sender").into()
    } else {
        names.join(", ")
    }
}

fn thread_view(t: &sms::Thread) -> ThreadView {
    ThreadView {
        id: t.id,
        date_ms: t.date_ms,
        title: title(t),
        snippet: clean(&t.snippet),
        people: t
            .people
            .iter()
            .map(|p| PersonView {
                address: clean(&p.address),
                name: p.name.as_deref().map(clean),
                photo: p.photo,
            })
            .collect(),
        unread: t.unread,
        count: t.count,
    }
}

fn message_view(m: &sms::Message, names: &HashMap<String, String>) -> MessageView {
    use wind_core::sms::MessageBox;
    MessageView {
        key: format!("{}{}", if m.mms { 'm' } else { 's' }, m.id),
        date_ms: m.date_ms,
        folder: match m.folder {
            MessageBox::Inbox => "in",
            MessageBox::Sent => "sent",
            MessageBox::Pending => "pending",
            MessageBox::Failed => "failed",
        },
        body: clean(&m.body),
        sender: m.address.as_ref().map(|a| clean(names.get(a).unwrap_or(a))),
        attachments: m.attachments,
        parts: m
            .parts
            .iter()
            .filter(|p| p.mime.starts_with("image/"))
            .map(|p| p.id)
            .collect(),
        delivery: m.delivery.map(|d| match d {
            sms::Delivery::Pending => "pending",
            sms::Delivery::Delivered => "delivered",
            sms::Delivery::Failed => "failed",
        }),
    }
}

fn status(state: &State) -> SmsStatus {
    SmsStatus {
        connected: state.session.is_some(),
        available: state.session.as_ref().is_some_and(offers),
    }
}

fn offers(session: &Session) -> bool {
    session
        .peer_capabilities()
        .iter()
        .any(|c| c.name() == "sms" && c.version() == 1)
}

fn emit_status(app: &tauri::AppHandle) {
    let s = status(&app.state::<Sms>().inner().0.lock().unwrap());
    let _ = app.emit_to(WINDOW, "sms-status", s);
}

/// A phone session started. Another phone's messages are dropped first.
pub fn attach(app: &tauri::AppHandle, session: Session) {
    {
        let mut state = app.state::<Sms>().inner().0.lock().unwrap();
        if state.phone.as_ref() != Some(&session.peer().key) {
            let watch = state.watch.take();
            *state = State {
                phone: Some(session.peer().key),
                ..State::default()
            };
            if let Some(w) = watch {
                w.abort();
            }
        }
        state.session = Some(session);
    }
    restart_watch(app);
    emit_status(app);
}

/// The session ended; what was read stays for this phone until another one connects or it is unpaired.
pub fn detach(app: &tauri::AppHandle) {
    {
        let mut state = app.state::<Sms>().inner().0.lock().unwrap();
        state.session = None;
        if let Some(w) = state.watch.take() {
            w.abort();
        }
    }
    emit_status(app);
}

/// The phone was unpaired: forget everything read from it.
pub fn forget(app: &tauri::AppHandle, key: &PublicKey) {
    let mut state = app.state::<Sms>().inner().0.lock().unwrap();
    if state.phone.as_ref() != Some(key) {
        return;
    }
    if let Some(w) = state.watch.take() {
        w.abort();
    }
    *state = State::default();
}

/// The phone changed what it accepts (its "SMS on the computer" setting or READ_SMS).
pub fn capabilities_changed(app: &tauri::AppHandle) {
    restart_watch(app);
    emit_status(app);
}

fn restart_watch(app: &tauri::AppHandle) {
    let mut state = app.state::<Sms>().inner().0.lock().unwrap();
    if let Some(w) = state.watch.take() {
        w.abort();
    }
    let Some(session) = state.session.clone().filter(offers) else {
        return;
    };
    let app = app.clone();
    state.watch = Some(tauri::async_runtime::spawn(async move {
        let Ok(mut changes) = sms::watch(&session).await else {
            return;
        };
        while changes.recv().await.is_some() {
            let _ = app.emit_to(WINDOW, "sms-changed", ());
        }
    }));
}

/// WN-100: the live phone session (the same one the SMS window uses), for other features' commands.
pub fn phone_session(app: &tauri::AppHandle) -> Option<Session> {
    app.state::<Sms>().inner().0.lock().unwrap().session.clone()
}

fn session(app: &tauri::AppHandle) -> Result<Session, String> {
    app.state::<Sms>()
        .inner()
        .0
        .lock()
        .unwrap()
        .session
        .clone()
        .ok_or_else(|| crate::i18n::tr("sms.not_connected").to_string())
}

fn ask_error(e: AskError) -> String {
    match e {
        AskError::Session(wind_core::session::SessionError::CapabilityMissing(_))
        | AskError::Status(Status::Denied) => crate::i18n::tr("sms.unavailable").into(),
        AskError::Status(Status::NotFound) => crate::i18n::tr("sms.thread_gone").into(),
        AskError::Status(Status::Busy) => crate::i18n::tr("sms.busy").into(),
        _ => crate::i18n::tr("sms.read_failed").into(),
    }
}

#[tauri::command]
pub fn sms_status(app: tauri::AppHandle) -> SmsStatus {
    status(&app.state::<Sms>().inner().0.lock().unwrap())
}

/// Threads from memory (`refresh` false, when there are any) or from the phone.
#[tauri::command]
pub async fn sms_threads(app: tauri::AppHandle, refresh: bool) -> Result<Vec<ThreadView>, String> {
    {
        let state = app.state::<Sms>().inner().0.lock().unwrap();
        if !refresh && !state.threads.is_empty() || state.session.is_none() {
            return Ok(state.threads.iter().map(thread_view).collect());
        }
    }
    let session = session(&app)?;
    let threads = sms::threads(&session, THREADS).await.map_err(ask_error)?;
    let mut state = app.state::<Sms>().inner().0.lock().unwrap();
    // Threads gone from the phone take their messages along.
    state
        .messages
        .retain(|id, _| threads.iter().any(|t| t.id == *id));
    state.threads = threads;
    Ok(state.threads.iter().map(thread_view).collect())
}

fn view(state: &State, thread: u64, cached: bool) -> ThreadMessages {
    let names: HashMap<String, String> = state
        .threads
        .iter()
        .find(|t| t.id == thread)
        .map(|t| {
            t.people
                .iter()
                .filter_map(|p| Some((p.address.clone(), p.name.clone()?)))
                .collect()
        })
        .unwrap_or_default();
    let (messages, complete) = state
        .messages
        .get(&thread)
        .map(|(m, c)| (m.values().map(|m| message_view(m, &names)).collect(), *c))
        .unwrap_or_default();
    ThreadMessages {
        messages,
        complete,
        cached,
    }
}

/// Merges an answer; true when it brought something new.
fn merge(state: &mut State, thread: u64, got: Vec<sms::Message>, oldest_reached: bool) -> bool {
    let (known, complete) = state.messages.entry(thread).or_default();
    let mut new = false;
    for m in got {
        new |= known.insert((m.date_ms, m.mms, m.id), m).is_none();
    }
    *complete |= oldest_reached;
    new
}

/// A thread: from memory plus what is newer on the phone (`since` = the newest known), or its newest page.
#[tauri::command]
pub async fn sms_thread(app: tauri::AppHandle, id: u64) -> Result<ThreadMessages, String> {
    let (newest, session) = {
        let state = app.state::<Sms>().inner().0.lock().unwrap();
        let newest = state
            .messages
            .get(&id)
            .and_then(|(m, _)| m.keys().next_back().map(|k| k.0));
        (newest, state.session.clone())
    };
    let Some(session) = session else {
        let state = app.state::<Sms>().inner().0.lock().unwrap();
        return if state.messages.contains_key(&id) {
            Ok(view(&state, id, true))
        } else {
            Err(crate::i18n::tr("sms.not_connected").into())
        };
    };
    let got = sms::messages(&session, id, None, newest, sms::MAX_MESSAGES.min(PAGE * 2))
        .await
        .map_err(ask_error)?;
    let mut state = app.state::<Sms>().inner().0.lock().unwrap();
    let full = newest.is_none() && got.len() < (PAGE * 2) as usize;
    merge(&mut state, id, got, full);
    Ok(view(&state, id, false))
}

/// The next older page of a thread.
#[tauri::command]
pub async fn sms_older(app: tauri::AppHandle, id: u64) -> Result<ThreadMessages, String> {
    let oldest = {
        let state = app.state::<Sms>().inner().0.lock().unwrap();
        state
            .messages
            .get(&id)
            .and_then(|(m, _)| m.keys().next().map(|k| k.0))
    };
    let session = session(&app)?;
    let got = sms::messages(&session, id, oldest, None, PAGE)
        .await
        .map_err(ask_error)?;
    let short = got.len() < PAGE as usize;
    let mut state = app.state::<Sms>().inner().0.lock().unwrap();
    let new = merge(&mut state, id, got, short);
    // A page of only known messages (many at one millisecond) can't go further.
    if !new {
        if let Some((_, complete)) = state.messages.get_mut(&id) {
            *complete = true;
        }
    }
    Ok(view(&state, id, false))
}

/// WN-56: the SIMs to send through, or an error saying sending is off on the phone.
#[tauri::command]
pub async fn sms_sims(app: tauri::AppHandle) -> Result<Vec<SimView>, String> {
    let session = session(&app)?;
    match sms::sims(&session).await {
        Ok(sims) => Ok(sims
            .into_iter()
            .map(|s| SimView {
                id: s.id,
                label: clean(&s.label),
                default: s.default,
            })
            .collect()),
        Err(AskError::Status(Status::Denied)) => Err(crate::i18n::tr("sms.send_off").into()),
        Err(AskError::Session(_)) | Err(AskError::Protocol) => {
            Err(crate::i18n::tr("sms.send_unsupported").into())
        }
        Err(e) => Err(ask_error(e)),
    }
}

/// WN-56: sends through the phone; returns an id, and `sms-send` events follow until `done`.
/// `image`: base64 JPEG/PNG (the window scales pictures down first).
#[tauri::command]
pub async fn sms_send(
    app: tauri::AppHandle,
    to: Vec<String>,
    body: String,
    sim: Option<u64>,
    image_mime: Option<String>,
    image: Option<String>,
) -> Result<u64, String> {
    let image = match (image_mime, image) {
        (Some(mime), Some(data)) => {
            let bytes = unbase64(&data).ok_or(crate::i18n::tr("sms.bad_image"))?;
            if bytes.len() > sms::MAX_IMAGE || !sms::image_matches(&mime, &bytes) {
                return Err(crate::i18n::tr("sms.image_too_large").into());
            }
            Some((mime, bytes))
        }
        _ => None,
    };
    let to: Vec<String> = to
        .iter()
        .map(|a| a.trim().to_owned())
        .filter(|a| !a.is_empty())
        .collect();
    if to.is_empty()
        || to.len() > sms::MAX_RECIPIENTS
        || to.iter().any(|a| a.len() > sms::MAX_ADDRESS)
    {
        return Err(crate::i18n::tr("sms.recipients").into());
    }
    if body.trim().is_empty() && image.is_none() {
        return Err(crate::i18n::tr("sms.empty").into());
    }
    if body.len() > sms::MAX_BODY {
        return Err(crate::i18n::tr("sms.too_long").into());
    }
    let session = session(&app)?;
    let message = sms::Outgoing {
        to,
        body,
        sim,
        image,
    };
    let mut states = sms::send(&session, message).await.map_err(ask_error)?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    tauri::async_runtime::spawn(async move {
        let mut last = "queued";
        while let Some(r) = states.recv().await {
            let event = match r {
                Ok(state) => {
                    last = match state {
                        sms::SendState::Queued => "queued",
                        sms::SendState::Sent => "sent",
                        sms::SendState::Delivered => "delivered",
                        sms::SendState::Failed => "failed",
                    };
                    SendEvent {
                        id,
                        state: last,
                        message: None,
                        done: false,
                    }
                }
                Err(e) => SendEvent {
                    id,
                    state: "refused",
                    message: Some(ask_error(e)),
                    done: true,
                },
            };
            let _ = app.emit_to(WINDOW, "sms-send", event);
        }
        let _ = app.emit_to(
            WINDOW,
            "sms-send",
            SendEvent {
                id,
                state: last,
                message: None,
                done: true,
            },
        );
    });
    Ok(id)
}

/// WN-56: an MMS picture as a `data:` URL (kept in memory like the messages).
#[tauri::command]
pub async fn sms_part(app: tauri::AppHandle, message: u64, part: u64) -> Result<String, String> {
    let key = format!("part:{message}:{part}");
    if let Some(Some(url)) = app
        .state::<Sms>()
        .inner()
        .0
        .lock()
        .unwrap()
        .photos
        .get(&key)
    {
        return Ok(url.clone());
    }
    let session = session(&app)?;
    let (mime, bytes) = sms::part(&session, message, part)
        .await
        .map_err(ask_error)?;
    if !sms::image_matches(&mime, &bytes) {
        return Err(crate::i18n::tr("sms.not_image").into());
    }
    let url = format!("data:{mime};base64,{}", base64(&bytes));
    app.state::<Sms>()
        .inner()
        .0
        .lock()
        .unwrap()
        .photos
        .insert(key, Some(url.clone()));
    Ok(url)
}

/// A contact photo as a `data:` URL (the window's CSP allows only those), or none.
#[tauri::command]
pub async fn sms_photo(app: tauri::AppHandle, address: String) -> Result<Option<String>, String> {
    if let Some(known) = app
        .state::<Sms>()
        .inner()
        .0
        .lock()
        .unwrap()
        .photos
        .get(&address)
    {
        return Ok(known.clone());
    }
    let session = session(&app)?;
    let png = sms::photo(&session, &address).await.map_err(ask_error)?;
    let url = png.map(|png| format!("data:image/png;base64,{}", base64(&png)));
    app.state::<Sms>()
        .inner()
        .0
        .lock()
        .unwrap()
        .photos
        .insert(address, url.clone());
    Ok(url)
}

/// Opens (or brings up) the "Messages" window. Async: building a window in a sync command deadlocks on Windows.
#[tauri::command]
pub async fn sms_open(app: tauri::AppHandle) -> Result<(), String> {
    open(&app).map_err(|e| e.to_string())
}

pub fn open(app: &tauri::AppHandle) -> tauri::Result<()> {
    if let Some(panel) = app.get_webview_window("panel") {
        let _ = panel.hide();
    }
    if let Some(window) = app.get_webview_window(WINDOW) {
        window.unminimize()?;
        window.show()?;
        return window.set_focus();
    }
    let window =
        tauri::WebviewWindowBuilder::new(app, WINDOW, tauri::WebviewUrl::App("sms.html".into()))
            .title(crate::i18n::tr("sms.window_title"))
            .inner_size(960.0, 680.0)
            .min_inner_size(560.0, 400.0)
            .build()?;
    window.set_focus()
}

/// Standard base64 (padding optional); none for anything else.
fn unbase64(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let text = text.trim_end_matches('=').as_bytes();
    if text.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    for chunk in text.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= value(*c)? << (18 - 6 * i);
        }
        out.extend_from_slice(&n.to_be_bytes()[1..chunk.len()]);
    }
    Some(out)
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(A[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: u64, date: u64) -> sms::Message {
        sms::Message {
            id,
            mms: false,
            date_ms: date,
            folder: sms::MessageBox::Inbox,
            body: format!("{id}"),
            address: None,
            read: true,
            attachments: 0,
            parts: vec![],
            delivery: None,
        }
    }

    #[test]
    fn unbase64_reverses_base64() {
        for data in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"\xff\xd8\xff\x00\x10",
        ] {
            assert_eq!(unbase64(&base64(data)).as_deref(), Some(data));
        }
        assert_eq!(unbase64("Zg"), Some(b"f".to_vec()));
        assert_eq!(unbase64("Z"), None);
        assert_eq!(unbase64("Zm9v!"), None);
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn clean_keeps_text_and_newlines() {
        assert_eq!(clean("a\u{202E}b\u{0007}c\nd\u{200F}"), "abc\nd");
        assert_eq!(clean("Zażółć 👋"), "Zażółć 👋"); // i18n: data
    }

    #[test]
    fn merging_dedups_the_boundary_and_keeps_order() {
        let mut state = State::default();
        assert!(merge(&mut state, 1, vec![m(3, 30), m(2, 20)], false));
        // `since` and `before` are inclusive: the boundary comes again.
        assert!(merge(&mut state, 1, vec![m(4, 40), m(3, 30)], false));
        assert!(!merge(&mut state, 1, vec![m(2, 20)], true));
        let v = view(&state, 1, false);
        assert_eq!(
            v.messages
                .iter()
                .map(|m| m.body.as_str())
                .collect::<Vec<_>>(),
            ["2", "3", "4"]
        );
        assert!(v.complete);
    }

    #[test]
    fn senders_show_contact_names() {
        let mut state = State::default();
        state.threads.push(sms::Thread {
            id: 1,
            date_ms: 0,
            snippet: String::new(),
            people: vec![sms::Person {
                address: "+48600".into(),
                name: Some("Ola".into()),
                photo: false,
            }],
            unread: 0,
            count: 1,
        });
        let mut got = m(1, 1);
        got.address = Some("+48600".into());
        merge(&mut state, 1, vec![got], true);
        assert_eq!(
            view(&state, 1, false).messages[0].sender.as_deref(),
            Some("Ola")
        );
        assert_eq!(title(&state.threads[0]), "Ola");
    }
}
