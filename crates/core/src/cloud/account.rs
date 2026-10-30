use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use super::client::{CloudError, CloudResult};
use super::envelope::{
    self, open, seal, signed_request, url_encode, EnvelopeClient, Identity, Tokens, COUNTER_WRAP,
    SIGNED_VERSION,
};
use super::types::{user_message, CloudProfile, Envelope, CODE_TOKEN_EXPIRED};

pub const PROFILE_ENDPOINT: &str = "userinfo2";
pub const CHANGE_PASSWORD_ENDPOINT: &str = "usercp";
pub const SEND_CODE_ENDPOINT: &str = "scmail";
pub const CHECK_CODE_ENDPOINT: &str = "usercpc";
pub const RESET_PASSWORD_ENDPOINT: &str = "fpmail";
pub const NAME_CHECK_ENDPOINT: &str = "usercheck";
pub const REGISTER_ENDPOINT: &str = "userregMail";
pub const CANCEL_ENDPOINT: &str = "userecancel";
pub const LANGUAGE_ENGLISH: i32 = 2;
pub const CODE_SIZE: &str = "6";
pub const CODE_NAME_TAKEN: i32 = 4012;
pub const CODE_NAME_FREE: i32 = 4025;
pub const CODE_MAIL_TAKEN: i32 = 4023;
pub const MAIL_TAKEN: &str =
    "this email already has a cloud account; sign in or reset its password";

const ACCEPT: &str = "text/html, image/gif, image/jpeg, *; q=.2, */*; q=.2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodePurpose {
    Forgot,
    Register,
}

impl CodePurpose {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Forgot => "fp",
            Self::Register => "re",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountProfile {
    pub id: Option<String>,
    pub username: Option<String>,
    pub mail: Option<String>,
    pub phone: Option<String>,
    pub nickname: Option<String>,
    pub register_time: Option<String>,
    pub pass_update_time: Option<String>,
    pub source: &'static str,
}

impl AccountProfile {
    pub fn from_userinfo(data: &Value) -> Self {
        Self {
            id: text_of(data, &["id", "userId"]),
            username: text_of(data, &["username", "userName"]),
            mail: text_of(data, &["mail", "email"]).map(|mail| mask_mail(&mail)),
            phone: text_of(data, &["phone"]).map(|phone| mask_phone(&phone)),
            nickname: text_of(data, &["nickname", "nickName"]),
            register_time: text_of(data, &["registerTime"]),
            pass_update_time: text_of(data, &["passUpdateTime"]),
            source: "cloud",
        }
    }

    pub fn from_portal(profile: &CloudProfile) -> Self {
        Self {
            username: profile
                .username
                .clone()
                .filter(|text| !text.trim().is_empty()),
            mail: profile
                .email
                .as_deref()
                .filter(|text| !text.trim().is_empty())
                .map(mask_mail),
            phone: profile
                .phone
                .as_deref()
                .filter(|text| !text.trim().is_empty())
                .map(mask_phone),
            source: "portal",
            ..Self::default()
        }
    }
}

fn text_of(data: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match data.get(*key)? {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

pub fn mask_mail(mail: &str) -> String {
    let mail = mail.trim();
    if mail.contains('*') {
        return mail.to_string();
    }
    match mail.split_once('@') {
        Some((local, domain)) => {
            let head: String = local.chars().take(2).collect();
            format!("{head}***@{domain}")
        }
        None => mask_phone(mail),
    }
}

pub fn mask_phone(phone: &str) -> String {
    let phone = phone.trim();
    if phone.contains('*') {
        return phone.to_string();
    }
    let chars: Vec<char> = phone.chars().collect();
    if chars.len() <= 4 {
        return "***".to_string();
    }
    let tail: String = chars[chars.len() - 3..].iter().collect();
    let head: String = chars[..2].iter().collect();
    format!("{head}***{tail}")
}

pub fn change_password_body(new_password: &str) -> String {
    let encoded = url_encode(new_password);
    format!("newPass={encoded}&confirmPass={encoded}")
}

pub fn send_code_body(mail: &str, purpose: CodePurpose) -> String {
    format!(
        "mail={}&type={}&language={LANGUAGE_ENGLISH}&username=&codeSize={CODE_SIZE}",
        url_encode(mail.trim()),
        purpose.wire()
    )
}

pub fn check_code_body(mail: &str, code: &str) -> String {
    format!(
        "phone=&mail={}&verCode={}",
        url_encode(mail.trim()),
        url_encode(code.trim())
    )
}

pub fn reset_password_body(user_id: &str, mail: &str, code: &str, new_password: &str) -> String {
    let encoded = url_encode(new_password);
    format!(
        "id={}&mail={}&verCode={}&newPass={encoded}&confirmPass={encoded}",
        url_encode(user_id.trim()),
        url_encode(mail.trim()),
        url_encode(code.trim())
    )
}

pub fn name_check_body(name: &str) -> String {
    format!("name={}", url_encode(name.trim()))
}

pub fn register_body(name: &str, password: &str, mail: &str, code: &str) -> String {
    let encoded = url_encode(password);
    format!(
        "name={}&password={encoded}&confirmPass={encoded}&mail={}&verCode={}",
        url_encode(name.trim()),
        url_encode(mail.trim()),
        url_encode(code.trim())
    )
}

pub fn cancel_body(code: &str) -> String {
    format!(
        "code={}&language={LANGUAGE_ENGLISH}&codeSize={CODE_SIZE}",
        url_encode(code.trim())
    )
}

pub fn name_available(reply: &Envelope) -> Option<bool> {
    match reply.code {
        CODE_NAME_FREE => Some(true),
        CODE_NAME_TAKEN => Some(false),
        _ if reply.is_ok() => Some(false),
        _ => None,
    }
}

pub fn user_id_of(data: &Value) -> Option<String> {
    text_of(data, &["userId", "id"])
}

pub fn ok_data(envelope: Envelope) -> CloudResult<Value> {
    if envelope.is_ok() {
        return Ok(envelope.data);
    }
    if envelope.code == CODE_TOKEN_EXPIRED {
        return Err(CloudError::Expired);
    }
    Err(CloudError::Rejected(user_message(
        envelope.code,
        envelope.msg.as_deref(),
    )))
}

fn required(value: &str, what: &str) -> CloudResult<()> {
    if value.trim().is_empty() {
        return Err(CloudError::Rejected(format!("{what} is required")));
    }
    Ok(())
}

pub struct AccountClient {
    http: reqwest::Client,
    base: String,
    identity: Identity,
    counter: Arc<AtomicU32>,
    envelope: EnvelopeClient,
}

impl AccountClient {
    pub fn with_base(identity: Identity, base: impl Into<String>) -> Self {
        let base = base.into().trim_end_matches('/').to_string();
        let base = if base.is_empty() {
            envelope::ENVELOPE_BASE.to_string()
        } else {
            base
        };
        let http = envelope::http();
        let counter = Arc::new(AtomicU32::new(0));
        let login = EnvelopeClient::with_shared(
            identity.clone(),
            base.clone(),
            http.clone(),
            counter.clone(),
        );
        Self {
            http,
            base,
            identity,
            counter,
            envelope: login,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub async fn raw(
        &self,
        endpoint: &str,
        body: Option<&str>,
        access: Option<&str>,
    ) -> CloudResult<Envelope> {
        if !self.identity.is_complete() {
            return Err(CloudError::Malformed(
                envelope::MISSING_CLOUD_IDENTITY.to_string(),
            ));
        }
        let counter = self.counter.fetch_add(1, Ordering::Relaxed) % (COUNTER_WRAP - 1) + 1;
        let signed = signed_request(&self.identity, counter, envelope::now_millis());
        let url = format!(
            "{}/{endpoint}/{}/{}/{}.rs",
            self.base, SIGNED_VERSION, signed.tsid, signed.sig
        );
        let mut request = self
            .http
            .post(url)
            .header("Accept", ACCEPT)
            .header("Accept-Charset", "utf-8")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("uuid", &self.identity.uuid)
            .header("appKey", &self.identity.app_key);
        if let Some(access) = access.map(str::trim).filter(|token| !token.is_empty()) {
            request = request.header("Authorization", access);
        }
        if let Some(plain) = body {
            request = request.body(seal(&signed.key, plain)?);
        }
        let response = request
            .send()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| CloudError::Unreachable(error.to_string()))?;
        if !status.is_success() {
            return Err(CloudError::Unreachable(format!("{endpoint} HTTP {status}")));
        }
        let plain = open(&signed.key, &text).unwrap_or(text);
        serde_json::from_str(&plain)
            .map_err(|error| CloudError::Malformed(format!("{endpoint}: {error}")))
    }

    pub async fn call(
        &self,
        endpoint: &str,
        body: Option<&str>,
        access: Option<&str>,
    ) -> CloudResult<Value> {
        ok_data(self.raw(endpoint, body, access).await?)
    }

    pub async fn profile(&self, access: &str) -> CloudResult<AccountProfile> {
        required(access, "a cloud session")?;
        let data = self.call(PROFILE_ENDPOINT, None, Some(access)).await?;
        Ok(AccountProfile::from_userinfo(&data))
    }

    pub async fn verify_password(&self, account: &str, password: &str) -> CloudResult<Tokens> {
        required(account, "the account")?;
        required(password, "the password")?;
        self.envelope.login_envelope(account, password).await
    }

    pub async fn change_password(&self, access: &str, new_password: &str) -> CloudResult<()> {
        required(new_password, "the new password")?;
        let body = change_password_body(new_password);
        self.call(CHANGE_PASSWORD_ENDPOINT, Some(&body), Some(access))
            .await
            .map(|_| ())
    }

    pub async fn send_code(&self, mail: &str, purpose: CodePurpose) -> CloudResult<Option<String>> {
        required(mail, "an email address")?;
        let body = send_code_body(mail, purpose);
        let reply = self.raw(SEND_CODE_ENDPOINT, Some(&body), None).await?;
        if purpose == CodePurpose::Register && reply.code == CODE_MAIL_TAKEN {
            return Err(CloudError::Rejected(MAIL_TAKEN.to_string()));
        }
        Ok(user_id_of(&ok_data(reply)?))
    }

    pub async fn check_code(&self, mail: &str, code: &str) -> CloudResult<()> {
        required(mail, "an email address")?;
        required(code, "the verification code")?;
        let body = check_code_body(mail, code);
        self.call(CHECK_CODE_ENDPOINT, Some(&body), None)
            .await
            .map(|_| ())
    }

    pub async fn reset_password(
        &self,
        user_id: &str,
        mail: &str,
        code: &str,
        new_password: &str,
    ) -> CloudResult<()> {
        required(user_id, "the account id from the code step")?;
        required(code, "the verification code")?;
        required(new_password, "the new password")?;
        let body = reset_password_body(user_id, mail, code, new_password);
        self.call(RESET_PASSWORD_ENDPOINT, Some(&body), None)
            .await
            .map(|_| ())
    }

    pub async fn name_check(&self, name: &str) -> CloudResult<Envelope> {
        required(name, "a username")?;
        let body = name_check_body(name);
        self.raw(NAME_CHECK_ENDPOINT, Some(&body), None).await
    }

    pub async fn register(
        &self,
        name: &str,
        password: &str,
        mail: &str,
        code: &str,
    ) -> CloudResult<()> {
        required(name, "a username")?;
        required(password, "a password")?;
        required(mail, "an email address")?;
        required(code, "the verification code")?;
        let body = register_body(name, password, mail, code);
        self.call(REGISTER_ENDPOINT, Some(&body), None)
            .await
            .map(|_| ())
    }

    pub async fn cancel(&self, access: &str, code: &str) -> CloudResult<()> {
        required(access, "a cloud session")?;
        let body = cancel_body(code);
        self.call(CANCEL_ENDPOINT, Some(&body), Some(access))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields(body: &str) -> Vec<(String, String)> {
        body.split('&')
            .map(|pair| {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                (key.to_string(), value.to_string())
            })
            .collect()
    }

    fn keys(body: &str) -> Vec<String> {
        fields(body).into_iter().map(|(key, _)| key).collect()
    }

    #[test]
    fn change_password_sends_the_new_password_twice_and_never_the_old_one() {
        let body = change_password_body("n3w&pass=x");
        assert_eq!(keys(&body), ["newPass", "confirmPass"]);
        let values: Vec<String> = fields(&body).into_iter().map(|(_, value)| value).collect();
        assert_eq!(values[0], values[1]);
        assert_eq!(values[0], "n3w%26pass%3Dx");
    }

    #[test]
    fn send_code_carries_purpose_english_and_six_digits() {
        let forgot = send_code_body(" a.b+c@example.invalid ", CodePurpose::Forgot);
        assert_eq!(
            forgot,
            "mail=a.b%2Bc%40example.invalid&type=fp&language=2&username=&codeSize=6"
        );
        let register = send_code_body("a@example.invalid", CodePurpose::Register);
        assert!(register.contains("&type=re&"));
        assert_eq!(
            keys(&register),
            ["mail", "type", "language", "username", "codeSize"]
        );
    }

    #[test]
    fn check_code_sends_an_empty_phone_and_the_mail() {
        assert_eq!(
            check_code_body("a@example.invalid", " 123456 "),
            "phone=&mail=a%40example.invalid&verCode=123456"
        );
    }

    #[test]
    fn reset_uses_the_user_id_from_the_code_step() {
        let body = reset_password_body("uid-1", "a@example.invalid", "123456", "pw");
        assert_eq!(
            body,
            "id=uid-1&mail=a%40example.invalid&verCode=123456&newPass=pw&confirmPass=pw"
        );
    }

    #[test]
    fn register_repeats_the_password_as_confirm() {
        let body = register_body("someone", "p@ss", "a@example.invalid", "654321");
        assert_eq!(
            body,
            "name=someone&password=p%40ss&confirmPass=p%40ss&mail=a%40example.invalid&verCode=654321"
        );
        assert_eq!(name_check_body(" some one "), "name=some%20one");
    }

    #[test]
    fn cancel_first_call_has_an_empty_code() {
        assert_eq!(cancel_body(""), "code=&language=2&codeSize=6");
        assert_eq!(cancel_body("112233"), "code=112233&language=2&codeSize=6");
    }

    #[test]
    fn profile_maps_userinfo_fields_and_numbers() {
        let data = json!({
            "id": "u-1",
            "userId": "ignored",
            "username": "someone",
            "mail": "so***@example.invalid",
            "phone": "",
            "nickname": "Some",
            "registerTime": "2024-01-02 03:04:05",
            "passUpdateTime": 1700000000
        });
        let profile = AccountProfile::from_userinfo(&data);
        assert_eq!(profile.id.as_deref(), Some("u-1"));
        assert_eq!(profile.username.as_deref(), Some("someone"));
        assert_eq!(profile.mail.as_deref(), Some("so***@example.invalid"));
        assert_eq!(profile.phone, None);
        assert_eq!(profile.nickname.as_deref(), Some("Some"));
        assert_eq!(
            profile.register_time.as_deref(),
            Some("2024-01-02 03:04:05")
        );
        assert_eq!(profile.pass_update_time.as_deref(), Some("1700000000"));
        assert_eq!(profile.source, "cloud");
        let wire = serde_json::to_value(&profile).expect("json");
        assert!(wire.get("registerTime").is_some());
        assert!(wire.get("passUpdateTime").is_some());
    }

    #[test]
    fn profile_masks_a_clear_mail_and_phone() {
        let data =
            json!({ "userId": "u-2", "mail": "someone@example.invalid", "phone": "0812345678" });
        let profile = AccountProfile::from_userinfo(&data);
        assert_eq!(profile.id.as_deref(), Some("u-2"));
        assert_eq!(profile.mail.as_deref(), Some("so***@example.invalid"));
        assert_eq!(profile.phone.as_deref(), Some("08***678"));
        let portal = AccountProfile::from_portal(&CloudProfile {
            username: Some("someone".to_string()),
            phone: None,
            email: Some("someone@example.invalid".to_string()),
        });
        assert_eq!(portal.mail.as_deref(), Some("so***@example.invalid"));
        assert_eq!(portal.source, "portal");
    }

    #[test]
    fn name_check_reads_the_vendor_codes() {
        let reply = |code: i32| Envelope {
            code,
            msg: None,
            data: Value::Null,
            extra: serde_json::Map::new(),
        };
        assert_eq!(name_available(&reply(4025)), Some(true));
        assert_eq!(name_available(&reply(4012)), Some(false));
        assert_eq!(name_available(&reply(2000)), Some(false));
        assert_eq!(name_available(&reply(4050)), None);
    }

    #[test]
    fn user_id_comes_from_the_send_code_reply() {
        assert_eq!(
            user_id_of(&json!({ "userId": "abc" })).as_deref(),
            Some("abc")
        );
        assert_eq!(user_id_of(&json!({})), None);
    }
}
