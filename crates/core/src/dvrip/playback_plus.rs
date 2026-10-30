use base64::Engine;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::Instant;

use super::playback::{
    stamp_seconds, PlaybackAction, PlaybackError, PlaybackFile, Record, RecordKind, Session,
    PLAYBACK_CLAIM, PLAYBACK_CONTROL,
};

pub const CALENDAR_MONTH: u16 = 1446;
pub const COMPRESS_PICTURE: u16 = 1448;
pub const COMPRESS_PICTURE_REPLY: u16 = 1449;
pub const BY_TIME_VALUE: u64 = 0x0400_0000;
pub const BY_TIME_NAME: &str = "/idea0/2015-10-20/001/00.00.00-00.00.09[H][@bff][0].h264";
pub const THUMB_WIDTH: u32 = 160;
pub const THUMB_HEIGHT: u32 = 100;
pub const NO_SUCH_FILE: i64 = 109;
pub const NO_FILE_OF_KIND: i64 = 119;
pub const AUDIO_RATE: u32 = 8000;
pub const AUDIO_CHUNK: usize = 3200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordFilter {
    All,
    Regular,
    Alarm,
}

impl RecordFilter {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "" | "all" | "*" => Some(RecordFilter::All),
            "regular" => Some(RecordFilter::Regular),
            "alarm" => Some(RecordFilter::Alarm),
            _ => None,
        }
    }

    pub fn events(self) -> &'static [&'static str] {
        match self {
            RecordFilter::All => &["*"],
            RecordFilter::Regular => &["R", "H"],
            RecordFilter::Alarm => &["M", "A"],
        }
    }

    pub fn wire(self) -> &'static str {
        match self {
            RecordFilter::All => "all",
            RecordFilter::Regular => "regular",
            RecordFilter::Alarm => "alarm",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayMode {
    #[default]
    ByName,
    ByTime,
}

impl PlayMode {
    pub fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.map(str::trim).unwrap_or("") {
            "" | "file" | "name" => Some(PlayMode::ByName),
            "time" => Some(PlayMode::ByTime),
            _ => None,
        }
    }

    pub fn wire(self) -> &'static str {
        match self {
            PlayMode::ByName => "ByName",
            PlayMode::ByTime => "ByTime",
        }
    }
}

pub fn record_letter(name: &str) -> Option<char> {
    let open = name.find('[')?;
    let mut rest = name[open + 1..].chars();
    let letter = rest.next()?;
    (rest.next() == Some(']')).then_some(letter)
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

pub fn calendar_body(session: u32, year: i32, month: u32) -> Value {
    json!({
        "Name": "OPSCalendar",
        "SessionID": format!("0x{session:08X}"),
        "OPSCalendar": {
            "Event": "*",
            "FileType": "h264",
            "Month": month,
            "Rev": "",
            "Year": year,
        },
    })
}

fn ret_of(reply: &Value) -> Option<i64> {
    reply.get("Ret").and_then(Value::as_i64)
}

fn rejected(ret: i64) -> PlaybackError {
    PlaybackError::Rejected {
        ret,
        reason: describe(ret),
    }
}

pub fn describe(ret: i64) -> &'static str {
    match ret {
        NO_SUCH_FILE => "the device holds no picture under that name",
        NO_FILE_OF_KIND => "the device holds no file of that kind",
        other => super::client::describe_ret(other),
    }
}

pub fn calendar_days(reply: &Value, year: i32, month: u32) -> Result<Vec<u32>, PlaybackError> {
    match ret_of(reply) {
        Some(100) => {}
        Some(ret) => return Err(rejected(ret)),
        None => {
            return Err(PlaybackError::Malformed(
                "the calendar reply carries no Ret".into(),
            ))
        }
    }
    let mask = reply
        .get("OPSCalendar")
        .and_then(|body| body.get("Mask"))
        .and_then(Value::as_u64)
        .ok_or_else(|| PlaybackError::Malformed("the calendar reply carries no Mask".into()))?;
    Ok((1..=days_in_month(year, month))
        .filter(|day| mask & (1u64 << (day - 1)) != 0)
        .collect())
}

pub fn by_time_body(
    session: u32,
    action: PlaybackAction,
    channel: u8,
    begin: &str,
    end: &str,
) -> Value {
    json!({
        "Name": "OPPlayBack",
        "SessionID": format!("0x{session:08X}"),
        "OPPlayBack": {
            "Action": action.wire(),
            "Parameter": {
                "Channel": channel,
                "FileName": BY_TIME_NAME,
                "PlayMode": "ByTime",
                "StreamType": 0,
                "TransMode": "TCP",
                "Value": BY_TIME_VALUE,
            },
            "StartTime": begin,
            "EndTime": end,
        },
    })
}

pub fn compress_picture_body(session: u32, name: &str) -> Value {
    json!({
        "Name": "OPCompressPic",
        "SessionID": format!("0x{session:08X}"),
        "OPCompressPic": {
            "Height": THUMB_HEIGHT,
            "IsGeo": 0,
            "PicName": name,
            "Width": THUMB_WIDTH,
        },
    })
}

pub fn picture_inside<'a>(
    file: &PlaybackFile,
    pictures: &'a [PlaybackFile],
) -> Option<&'a PlaybackFile> {
    pictures
        .iter()
        .filter(|picture| {
            picture.begin >= file.begin && (picture.begin < file.end || file.begin == file.end)
        })
        .min_by(|left, right| left.begin.cmp(&right.begin))
}

pub fn jpeg_data_url(jpeg: &[u8]) -> String {
    format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    )
}

pub fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() > 4 && bytes.starts_with(&[0xff, 0xd8]) && bytes.ends_with(&[0xff, 0xd9])
}

pub fn picture_reply(payload: &[u8]) -> Result<Vec<u8>, PlaybackError> {
    if is_jpeg(payload) {
        return Ok(payload.to_vec());
    }
    let mut end = payload.len();
    while end > 0 && matches!(payload[end - 1], 0 | b'\n' | b'\r') {
        end -= 1;
    }
    match serde_json::from_slice::<Value>(&payload[..end]) {
        Ok(reply) => match ret_of(&reply) {
            Some(ret) if ret != 100 => Err(rejected(ret)),
            _ => Err(PlaybackError::Malformed(
                "the picture reply carried JSON but no JPEG".into(),
            )),
        },
        Err(_) => Err(PlaybackError::Malformed(format!(
            "the picture reply is neither JPEG nor JSON ({} bytes)",
            payload.len()
        ))),
    }
}

pub fn stamp_text(raw: u32) -> Option<String> {
    stamp_seconds(raw)?;
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        (raw >> 26) + 2000,
        (raw >> 22) & 0xf,
        (raw >> 17) & 0x1f,
        (raw >> 12) & 0x1f,
        (raw >> 6) & 0x3f,
        raw & 0x3f
    ))
}

#[derive(Debug, Default)]
pub struct PlaybackSide {
    audio: Vec<u8>,
    audio_frame: usize,
    out: Vec<Value>,
    clocks: usize,
    audio_records: usize,
}

impl PlaybackSide {
    pub fn observe(&mut self, record: &Record, frame: usize) {
        match record.kind {
            RecordKind::Key => {
                if let Some(at) = record.stamp.and_then(stamp_text) {
                    self.clocks += 1;
                    self.out
                        .push(json!({ "type": "clock", "frame": frame, "at": at }));
                }
            }
            RecordKind::Pad if !record.bytes.is_empty() => {
                if self.audio.is_empty() {
                    self.audio_frame = frame;
                }
                self.audio.extend_from_slice(&record.bytes);
                self.audio_records += 1;
                if self.audio.len() >= AUDIO_CHUNK {
                    self.flush();
                }
            }
            _ => {}
        }
    }

    pub fn flush(&mut self) {
        if self.audio.is_empty() {
            return;
        }
        let data = base64::engine::general_purpose::STANDARD.encode(&self.audio);
        self.out.push(json!({
            "type": "audio",
            "frame": self.audio_frame,
            "codec": "alaw",
            "rate": AUDIO_RATE,
            "data": data,
        }));
        self.audio.clear();
    }

    pub fn take(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.out)
    }

    pub fn clocks(&self) -> usize {
        self.clocks
    }

    pub fn audio_records(&self) -> usize {
        self.audio_records
    }
}

impl Session {
    pub async fn calendar(&mut self, year: i32, month: u32) -> Result<Vec<u32>, PlaybackError> {
        let reply = self
            .request(CALENDAR_MONTH, calendar_body(self.session(), year, month))
            .await?;
        calendar_days(&reply, year, month)
    }

    pub async fn files_by(
        &mut self,
        channel: u8,
        begin: &str,
        end: &str,
        kind: &str,
        filter: RecordFilter,
    ) -> Result<Vec<PlaybackFile>, PlaybackError> {
        let mut found: Vec<PlaybackFile> = Vec::new();
        for event in filter.events() {
            match self.files_of(channel, begin, end, kind, event).await {
                Ok(files) => found.extend(files),
                Err(PlaybackError::Rejected {
                    ret: NO_FILE_OF_KIND,
                    ..
                }) => {}
                Err(error) => return Err(error),
            }
        }
        if filter.events().len() > 1 {
            found.sort_by(|left, right| left.begin.cmp(&right.begin));
            found.dedup_by(|left, right| left.name == right.name);
        }
        Ok(found)
    }

    pub async fn claim_by_time(
        &mut self,
        channel: u8,
        begin: &str,
        end: &str,
    ) -> Result<i64, PlaybackError> {
        let body = by_time_body(self.session(), PlaybackAction::Claim, channel, begin, end);
        let reply = self.request(PLAYBACK_CLAIM, body).await?;
        match ret_of(&reply) {
            Some(ret) if ret == 100 || ret == 150 => Ok(ret),
            Some(ret) => Err(rejected(ret)),
            None => Err(PlaybackError::Malformed(
                "the claim reply carries no Ret".into(),
            )),
        }
    }

    pub async fn start_by_time(
        &mut self,
        channel: u8,
        begin: &str,
        end: &str,
    ) -> Result<(), PlaybackError> {
        let body = by_time_body(self.session(), PlaybackAction::Start, channel, begin, end);
        self.send_json_plain(PLAYBACK_CONTROL, &body).await
    }

    pub async fn stop_by_time(
        &mut self,
        channel: u8,
        begin: &str,
        end: &str,
    ) -> Result<(), PlaybackError> {
        let body = by_time_body(self.session(), PlaybackAction::Stop, channel, begin, end);
        self.send_json(PLAYBACK_CONTROL, &body).await
    }

    pub async fn claim_in(
        &mut self,
        mode: PlayMode,
        file: &PlaybackFile,
        channel: u8,
    ) -> Result<i64, PlaybackError> {
        match mode {
            PlayMode::ByName => self.claim(file, channel).await,
            PlayMode::ByTime => self.claim_by_time(channel, &file.begin, &file.end).await,
        }
    }

    pub async fn start_in(
        &mut self,
        mode: PlayMode,
        file: &PlaybackFile,
        channel: u8,
    ) -> Result<(), PlaybackError> {
        match mode {
            PlayMode::ByName => self.start(file, channel).await,
            PlayMode::ByTime => self.start_by_time(channel, &file.begin, &file.end).await,
        }
    }

    pub async fn stop_in(
        &mut self,
        mode: PlayMode,
        file: &PlaybackFile,
        channel: u8,
    ) -> Result<(), PlaybackError> {
        match mode {
            PlayMode::ByName => self.stop(file, channel).await,
            PlayMode::ByTime => self.stop_by_time(channel, &file.begin, &file.end).await,
        }
    }

    pub async fn compressed_picture(
        &mut self,
        name: &str,
        wait: Duration,
    ) -> Result<Vec<u8>, PlaybackError> {
        self.send_json(
            COMPRESS_PICTURE,
            &compress_picture_body(self.session(), name),
        )
        .await?;
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(PlaybackError::NoReply(wait));
            }
            match self.frame(left).await? {
                Some(frame) if frame.id == COMPRESS_PICTURE_REPLY => {
                    return picture_reply(&frame.payload)
                }
                Some(_) => continue,
                None => return Err(PlaybackError::NoReply(wait)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dvrip::fake::{json_reply, serve_scripted};
    use tokio::net::TcpStream;

    fn file(name: &str, begin: &str, end: &str) -> PlaybackFile {
        PlaybackFile {
            name: name.into(),
            begin: begin.into(),
            end: end.into(),
            bytes: 0,
            disk: None,
        }
    }

    #[test]
    fn the_calendar_request_carries_the_fields_the_app_sends() {
        let body = calendar_body(0x287, 2026, 10);
        assert_eq!(
            body,
            json!({
                "Name": "OPSCalendar",
                "SessionID": "0x00000287",
                "OPSCalendar": {"Event": "*", "FileType": "h264", "Month": 10, "Rev": "", "Year": 2026},
            })
        );
    }

    #[test]
    fn the_captured_october_mask_marks_the_twenty_fifth_to_the_thirtieth() {
        let reply =
            json!({"Name": "OPSCalendar", "OPSCalendar": {"Mask": 1056964608u64}, "Ret": 100});
        assert_eq!(
            calendar_days(&reply, 2026, 10).unwrap(),
            vec![25, 26, 27, 28, 29, 30]
        );
    }

    #[test]
    fn bit_zero_is_the_first_and_bits_past_the_month_are_dropped() {
        let reply = json!({"OPSCalendar": {"Mask": 0x4000_0001u64}, "Ret": 100});
        assert_eq!(calendar_days(&reply, 2026, 9).unwrap(), vec![1]);
        assert_eq!(calendar_days(&reply, 2026, 10).unwrap(), vec![1, 31]);
        let leap = json!({"OPSCalendar": {"Mask": 0x1000_0000u64}, "Ret": 100});
        assert_eq!(calendar_days(&leap, 2028, 2).unwrap(), vec![29]);
        assert!(calendar_days(&leap, 2026, 2).unwrap().is_empty());
    }

    #[test]
    fn a_refused_calendar_is_an_error_not_an_empty_month() {
        let reply = json!({"Ret": 103});
        assert!(matches!(
            calendar_days(&reply, 2026, 10),
            Err(PlaybackError::Rejected { ret: 103, .. })
        ));
        let no_mask = json!({"Ret": 100});
        assert!(matches!(
            calendar_days(&no_mask, 2026, 10),
            Err(PlaybackError::Malformed(_))
        ));
    }

    #[test]
    fn a_play_by_time_claim_matches_the_captured_app_request() {
        let body = by_time_body(
            0x157,
            PlaybackAction::Claim,
            0,
            "2026-10-27 21:20:00",
            "2026-10-27 23:59:59",
        );
        assert_eq!(
            body,
            json!({
                "Name": "OPPlayBack",
                "SessionID": "0x00000157",
                "OPPlayBack": {
                    "Action": "Claim",
                    "Parameter": {
                        "Channel": 0,
                        "FileName": "/idea0/2015-10-20/001/00.00.00-00.00.09[H][@bff][0].h264",
                        "PlayMode": "ByTime",
                        "StreamType": 0,
                        "TransMode": "TCP",
                        "Value": 67108864u64,
                    },
                    "StartTime": "2026-10-27 21:20:00",
                    "EndTime": "2026-10-27 23:59:59",
                },
            })
        );
    }

    #[test]
    fn the_thumbnail_request_names_the_picture_and_the_app_size() {
        let body =
            compress_picture_body(1, "/idea1/2026-10-30/001/00.03.32-00.03.32[M][@1b][0].jpg");
        assert_eq!(body["Name"], "OPCompressPic");
        assert_eq!(
            body["OPCompressPic"]["PicName"],
            "/idea1/2026-10-30/001/00.03.32-00.03.32[M][@1b][0].jpg"
        );
        assert_eq!(body["OPCompressPic"]["Width"], 160);
        assert_eq!(body["OPCompressPic"]["Height"], 100);
        assert_eq!(body["OPCompressPic"]["IsGeo"], 0);
    }

    #[test]
    fn a_picture_reply_is_a_jpeg_or_the_device_reason() {
        let jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0xff, 0xd9];
        assert_eq!(picture_reply(&jpeg).unwrap(), jpeg);
        let refused = br#"{ "Name" : "", "Ret" : 109, "SessionID" : "0x0000028a" }"#.to_vec();
        match picture_reply(&refused) {
            Err(PlaybackError::Rejected { ret: 109, reason }) => {
                assert!(reason.contains("no picture"))
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            picture_reply(&[0xff, 0xd8, 1, 2, 3]),
            Err(PlaybackError::Malformed(_))
        ));
    }

    #[test]
    fn the_first_picture_inside_a_clip_is_its_thumbnail() {
        let clip = file(
            "/idea0/x[M].h264",
            "2026-10-30 00:03:32",
            "2026-10-30 00:03:57",
        );
        let pictures = vec![
            file("/idea1/b.jpg", "2026-10-30 00:03:50", "2026-10-30 00:03:50"),
            file("/idea1/a.jpg", "2026-10-30 00:03:32", "2026-10-30 00:03:32"),
            file("/idea1/c.jpg", "2026-10-30 00:04:10", "2026-10-30 00:04:10"),
        ];
        assert_eq!(
            picture_inside(&clip, &pictures).map(|p| p.name.as_str()),
            Some("/idea1/a.jpg")
        );
        let other = file(
            "/idea0/y[R].h264",
            "2026-10-30 01:00:00",
            "2026-10-30 01:05:00",
        );
        assert!(picture_inside(&other, &pictures).is_none());
        let before = file(
            "/idea0/z[R].h264",
            "2026-10-30 00:00:00",
            "2026-10-30 00:03:32",
        );
        assert!(picture_inside(&before, &pictures).is_none());
    }

    #[test]
    fn a_packed_stamp_prints_as_device_time() {
        assert_eq!(
            stamp_text(0x6AB36607).as_deref(),
            Some("2026-10-25 22:24:07")
        );
        assert_eq!(stamp_text(0), None);
    }

    #[test]
    fn keys_become_clock_marks_and_audio_is_sent_in_chunks() {
        let mut side = PlaybackSide::default();
        side.observe(
            &Record {
                kind: RecordKind::Key,
                stamp: Some(0x6AB36607),
                bytes: vec![1],
            },
            36,
        );
        for index in 0..10 {
            side.observe(
                &Record {
                    kind: RecordKind::Pad,
                    stamp: None,
                    bytes: vec![0xd5; 320],
                },
                40 + index,
            );
        }
        side.observe(
            &Record {
                kind: RecordKind::Delta,
                stamp: None,
                bytes: vec![2],
            },
            50,
        );
        let out = side.take();
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0],
            json!({"type": "clock", "frame": 36, "at": "2026-10-25 22:24:07"})
        );
        assert_eq!(out[1]["type"], "audio");
        assert_eq!(out[1]["frame"], 40);
        assert_eq!(out[1]["rate"], 8000);
        let data = base64::engine::general_purpose::STANDARD
            .decode(out[1]["data"].as_str().unwrap())
            .unwrap();
        assert_eq!(data, vec![0xd5; 3200]);
        assert!(side.take().is_empty());
        side.observe(
            &Record {
                kind: RecordKind::Pad,
                stamp: None,
                bytes: vec![0xd5; 320],
            },
            60,
        );
        assert!(side.take().is_empty());
        side.flush();
        assert_eq!(side.take()[0]["frame"], 60);
        assert_eq!(side.audio_records(), 11);
        assert_eq!(side.clocks(), 1);
    }

    #[test]
    fn filters_map_to_the_device_event_letters() {
        assert_eq!(RecordFilter::parse("all").unwrap().events(), &["*"]);
        assert_eq!(
            RecordFilter::parse("regular").unwrap().events(),
            &["R", "H"]
        );
        assert_eq!(RecordFilter::parse("alarm").unwrap().events(), &["M", "A"]);
        assert_eq!(RecordFilter::parse("bogus"), None);
        assert_eq!(
            record_letter("/idea0/2026-10-30/001/00.03.32-00.03.57[M][@162a][0].h264"),
            Some('M')
        );
        assert_eq!(record_letter("/idea0/plain.h264"), None);
        assert_eq!(PlayMode::parse(Some("time")), Some(PlayMode::ByTime));
        assert_eq!(PlayMode::parse(None), Some(PlayMode::ByName));
        assert_eq!(PlayMode::parse(Some("later")), None);
    }

    async fn session_on(address: &str) -> Session {
        Session::open(TcpStream::connect(address).await.unwrap())
    }

    #[tokio::test]
    async fn the_calendar_goes_out_as_message_1446_and_reads_the_mask() {
        let (address, seen) = serve_scripted(|_, id, _| {
            if id == CALENDAR_MONTH {
                vec![(1447, json_reply(json!({"Name": "OPSCalendar", "OPSCalendar": {"Mask": 0x3F00_0000u64}, "Ret": 100})))]
            } else {
                Vec::new()
            }
        })
        .await;
        let mut session = session_on(&address).await;
        assert_eq!(
            session.calendar(2026, 10).await.unwrap(),
            vec![25, 26, 27, 28, 29, 30]
        );
        drop(session);
        let seen = seen.await.unwrap();
        assert_eq!(seen[0].1, CALENDAR_MONTH);
        let sent: Value = serde_json::from_slice(&seen[0].2[..seen[0].2.len() - 2]).unwrap();
        assert_eq!(sent["OPSCalendar"]["Month"], 10);
        assert_eq!(sent["OPSCalendar"]["Year"], 2026);
    }

    #[tokio::test]
    async fn an_alarm_filter_asks_for_motion_and_alarm_and_treats_119_as_none() {
        let (address, seen) = serve_scripted(|_, _, body| {
            let text = String::from_utf8_lossy(body);
            let reply = if text.contains(r#""Event":"M""#) {
                json!({"Ret": 100, "OPFileQuery": [
                    {"FileName": "/idea0/2026-10-30/001/00.03.32-00.03.57[M][@162a][0].h264", "BeginTime": "2026-10-30 00:03:32", "EndTime": "2026-10-30 00:03:57", "FileLength": "0x7cc"}
                ]})
            } else {
                json!({"Ret": 119})
            };
            vec![(1441, json_reply(reply))]
        })
        .await;
        let mut session = session_on(&address).await;
        let found = session
            .files_by(
                0,
                "2026-10-30 00:00:00",
                "2026-10-30 23:59:59",
                "h264",
                RecordFilter::Alarm,
            )
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].bytes, 0x7cc * 1024);
        drop(session);
        let events: Vec<String> = seen
            .await
            .unwrap()
            .iter()
            .map(|(_, _, body)| String::from_utf8_lossy(body).into_owned())
            .collect();
        assert!(events[0].contains(r#""Event":"M""#));
        assert!(events[1].contains(r#""Event":"A""#));
    }

    #[tokio::test]
    async fn a_refused_file_query_still_fails_the_listing() {
        let (address, _seen) =
            serve_scripted(|_, _, _| vec![(1441, json_reply(json!({"Ret": 103})))]).await;
        let mut session = session_on(&address).await;
        let error = session
            .files_by(
                0,
                "2026-10-30 00:00:00",
                "2026-10-30 23:59:59",
                "h264",
                RecordFilter::Regular,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, PlaybackError::Rejected { ret: 103, .. }));
    }

    #[tokio::test]
    async fn a_thumbnail_skips_media_frames_and_returns_the_jpeg_reply() {
        let jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 9, 9, 0xff, 0xd9];
        let picture = jpeg.clone();
        let (address, seen) = serve_scripted(move |_, id, _| {
            if id == COMPRESS_PICTURE {
                vec![
                    (0x058e, vec![0, 0, 1, 0xfd]),
                    (COMPRESS_PICTURE_REPLY, picture.clone()),
                ]
            } else {
                Vec::new()
            }
        })
        .await;
        let mut session = session_on(&address).await;
        let got = session
            .compressed_picture("/idea1/a.jpg", Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(got, jpeg);
        drop(session);
        assert_eq!(seen.await.unwrap()[0].1, COMPRESS_PICTURE);
    }

    #[tokio::test]
    async fn a_silent_device_ends_the_thumbnail_wait_at_its_deadline() {
        let (address, _seen) = serve_scripted(|_, _, _| Vec::new()).await;
        let mut session = session_on(&address).await;
        let started = std::time::Instant::now();
        let error = session
            .compressed_picture("/idea1/a.jpg", Duration::from_millis(300))
            .await
            .unwrap_err();
        assert!(matches!(error, PlaybackError::NoReply(_)));
        assert!(started.elapsed() < Duration::from_millis(1500));
    }

    #[tokio::test]
    async fn play_by_time_claims_encrypted_and_starts_plain_with_the_time_span() {
        let (address, seen) = serve_scripted(|_, id, _| {
            if id == PLAYBACK_CLAIM {
                vec![(1425, json_reply(json!({"Name": "OPPlayBack", "Ret": 100})))]
            } else {
                Vec::new()
            }
        })
        .await;
        let mut session = session_on(&address).await;
        let span = file(BY_TIME_NAME, "2026-10-30 03:09:40", "2026-10-30 23:59:59");
        assert_eq!(
            session.claim_in(PlayMode::ByTime, &span, 0).await.unwrap(),
            100
        );
        session.start_in(PlayMode::ByTime, &span, 0).await.unwrap();
        session.stop_in(PlayMode::ByTime, &span, 0).await.unwrap();
        drop(session);
        let seen = seen.await.unwrap();
        let actions: Vec<(u16, Value)> = seen
            .iter()
            .map(|(_, id, body)| {
                let mut end = body.len();
                while end > 0 && matches!(body[end - 1], 0 | b'\n') {
                    end -= 1;
                }
                (*id, serde_json::from_slice(&body[..end]).unwrap())
            })
            .collect();
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0].0, PLAYBACK_CLAIM);
        assert_eq!(actions[1].0, PLAYBACK_CONTROL);
        assert_eq!(actions[2].0, PLAYBACK_CONTROL);
        for ((_, body), action) in actions.iter().zip(["Claim", "Start", "Stop"]) {
            assert_eq!(body["OPPlayBack"]["Action"], action);
            assert_eq!(body["OPPlayBack"]["Parameter"]["PlayMode"], "ByTime");
            assert_eq!(body["OPPlayBack"]["StartTime"], "2026-10-30 03:09:40");
        }
    }
}
