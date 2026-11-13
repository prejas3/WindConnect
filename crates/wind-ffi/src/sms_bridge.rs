//! CCS/1 M:WN-55 the phone's SMS for the PC (sms/1, §9.11): Kotlin reads the content providers through [`SmsSource`],
//! the core answers the PC's `sms` streams. RULE:`sms/1` is announced only while the person's setting is on and
//! READ_SMS is granted (Kotlin decides, [`crate::CoreClient::set_sms_enabled`]); texts and numbers are never logged.
use std::sync::Arc;

use wind_core::sms::{self, Delivery, MessageBox, SendState, Status};

/// WN-55: someone in a thread; `name` and `photo` need READ_CONTACTS.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SmsPerson {
    pub address: String,
    pub name: Option<String>,
    pub photo: bool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SmsThread {
    pub id: u64,
    pub date_ms: u64,
    pub snippet: String,
    pub people: Vec<SmsPerson>,
    pub unread: u32,
    pub count: u32,
}

/// `folder`: 0 received, 1 sent, 2 being sent, 3 failed.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SmsMessage {
    pub id: u64,
    pub mms: bool,
    pub date_ms: u64,
    pub folder: u8,
    pub body: String,
    pub address: Option<String>,
    pub read: bool,
    pub attachments: u32,
    /// WN-56: MMS parts the PC may fetch (pictures).
    pub parts: Vec<SmsPartInfo>,
    /// WN-56: an SMS's delivery report: 0 pending, 1 delivered, 2 failed; none when not asked for.
    pub delivery: Option<u8>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SmsPartInfo {
    pub id: u64,
    pub mime: String,
}

/// WN-56: a SIM to send through.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SmsSim {
    pub id: u64,
    pub label: String,
    pub slot: u32,
    pub default_sim: bool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SmsSims {
    pub status: u8,
    pub items: Vec<SmsSim>,
}

/// WN-56: a message from the PC. `image_mime` is image/jpeg or image/png, its bytes already checked by the core.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SmsOutgoing {
    pub to: Vec<String>,
    pub body: String,
    pub sim: Option<u64>,
    pub image_mime: Option<String>,
    pub image: Option<Vec<u8>>,
}

/// WN-56: an MMS part for the PC; status 2 when there is none.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SmsPart {
    pub status: u8,
    pub mime: String,
    pub data: Vec<u8>,
}

/// `status`: 0 ok, 1 denied (no READ_SMS or turned off), 2 not found, 3 busy, 4 failed.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SmsThreads {
    pub status: u8,
    pub items: Vec<SmsThread>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SmsMessages {
    pub status: u8,
    pub items: Vec<SmsMessage>,
}

/// `png`: at most 64 KiB; status 2 when the address has no photo.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SmsPhoto {
    pub status: u8,
    pub png: Vec<u8>,
}

/// WN-55: the phone's SMS store, called off the UI thread (may block on the content provider). Never throws: a
/// failure is status 4. Never re-enter the core from it.
#[uniffi::export(callback_interface)]
pub trait SmsSource: Send + Sync {
    fn threads(&self, limit: u32) -> SmsThreads;
    /// Newest first; `before` and `since` are inclusive dates in ms.
    fn messages(
        &self,
        thread: u64,
        before: Option<u64>,
        since: Option<u64>,
        limit: u32,
    ) -> SmsMessages;
    fn photo(&self, address: String) -> SmsPhoto;
    /// WN-56: status 1 while sending is off (the setting or SEND_SMS).
    fn sims(&self) -> SmsSims;
    /// WN-56: hands the message to the system and returns at once: 0 taken, else a status. Its states follow
    /// through [`crate::CoreClient::sms_send_state`] with `token`.
    fn send(&self, token: u64, message: SmsOutgoing) -> u8;
    fn part(&self, message: u64, part: u64) -> SmsPart;
}

pub(crate) struct PlatformSms(pub Box<dyn SmsSource>);

fn status(code: u8) -> Status {
    Status::from_code(code.into())
}

fn result<T>(code: u8, value: T) -> Result<T, Status> {
    match status(code) {
        Status::Ok => Ok(value),
        s => Err(s),
    }
}

impl sms::Source for PlatformSms {
    fn threads(&self, limit: u32) -> Result<Vec<sms::Thread>, Status> {
        let answer = self.0.threads(limit);
        result(answer.status, ())?;
        Ok(answer
            .items
            .into_iter()
            .map(|t| sms::Thread {
                id: t.id,
                date_ms: t.date_ms,
                snippet: t.snippet,
                people: t
                    .people
                    .into_iter()
                    .map(|p| sms::Person {
                        address: p.address,
                        name: p.name.filter(|n| !n.is_empty()),
                        photo: p.photo,
                    })
                    .collect(),
                unread: t.unread,
                count: t.count,
            })
            .collect())
    }
    fn messages(
        &self,
        thread: u64,
        before: Option<u64>,
        since: Option<u64>,
        limit: u32,
    ) -> Result<Vec<sms::Message>, Status> {
        let answer = self.0.messages(thread, before, since, limit);
        result(answer.status, ())?;
        Ok(answer
            .items
            .into_iter()
            .map(|m| sms::Message {
                id: m.id,
                mms: m.mms,
                date_ms: m.date_ms,
                folder: MessageBox::from_code(m.folder.into()),
                body: m.body,
                address: m.address,
                read: m.read,
                attachments: m.attachments,
                parts: m
                    .parts
                    .into_iter()
                    .map(|p| sms::Part {
                        id: p.id,
                        mime: p.mime,
                    })
                    .collect(),
                delivery: m.delivery.map(|d| Delivery::from_code(d.into())),
            })
            .collect())
    }
    fn photo(&self, address: &str) -> Result<Vec<u8>, Status> {
        let answer = self.0.photo(address.to_owned());
        result(answer.status, answer.png)
    }
    fn sims(&self) -> Result<Vec<sms::Sim>, Status> {
        let answer = self.0.sims();
        result(answer.status, ())?;
        Ok(answer
            .items
            .into_iter()
            .map(|s| sms::Sim {
                id: s.id,
                label: s.label,
                slot: s.slot,
                default: s.default_sim,
            })
            .collect())
    }
    fn send(&self, token: u64, message: sms::Outgoing) -> Result<(), Status> {
        let (image_mime, image) = match message.image {
            Some((m, b)) => (Some(m), Some(b)),
            None => (None, None),
        };
        let code = self.0.send(
            token,
            SmsOutgoing {
                to: message.to,
                body: message.body,
                sim: message.sim,
                image_mime,
                image,
            },
        );
        result(code, ())
    }
    fn part(&self, message: u64, part: u64) -> Result<(String, Vec<u8>), Status> {
        let answer = self.0.part(message, part);
        result(answer.status, (answer.mime, answer.data))
    }
}

/// `state`: 0 queued, 1 sent, 2 delivered, 3 failed.
pub(crate) fn send_state(code: u8) -> SendState {
    SendState::from_code(code.into())
}

pub(crate) type SmsRoute = (
    Arc<sms::Server>,
    tokio::sync::mpsc::Receiver<wind_core::session::SmsStream>,
);
