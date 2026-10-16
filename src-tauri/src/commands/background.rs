// 鸣潮官网 CDN 鉴权密钥可能随官网更新而轮换，随 SSMT 更新维护此常量。
const WWMI_BACKGROUND_AUTH_SECRET: &str = "HistL0gnHewqHEXdqaJq";

fn sign_wwmi_url(raw_url: &str, nonce: &str, timestamp: i64) -> Result<String, String> {
    let mut url = reqwest::Url::parse(raw_url).map_err(|error| error.to_string())?;
    if url.scheme() != "https" || url.host_str() != Some("mc.kurogames.com") {
        return Err("鸣潮背景签名仅支持官网 HTTPS 资源".into());
    }
    if nonce.len() != 10 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("鸣潮背景签名需要 5 字节十六进制随机数".into());
    }
    let payload = format!(
        "{}-{timestamp}-{nonce}-0-{WWMI_BACKGROUND_AUTH_SECRET}",
        url.path()
    );
    let hash = md5::compute(payload);
    let auth_key = format!("{timestamp}-{nonce}-0-{hash:x}");
    let mut query: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != "auth_key")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    query.push(("auth_key".into(), auth_key));
    url.query_pairs_mut().clear().extend_pairs(query);
    Ok(url.into())
}

#[tauri::command]
pub fn sign_wwmi_background_url(url: String, nonce: String) -> Result<String, String> {
    sign_wwmi_url(&url, &nonce, chrono::Utc::now().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIDEO_URL: &str = "https://mc.kurogames.com/website-preface/video/bg/bg-video.mp4";

    #[test]
    fn signature_matches_node_crypto_reference() {
        let signed = sign_wwmi_url(VIDEO_URL, "0123456789", 1_700_000_000).unwrap();
        assert_eq!(
            signed,
            format!(
                "{VIDEO_URL}?auth_key=1700000000-0123456789-0-9231480da32939f907d44671d01b262f"
            )
        );
    }

    #[test]
    fn refresh_replaces_expired_auth_key_and_preserves_other_query_pairs() {
        let signed = sign_wwmi_url(
            &format!("{VIDEO_URL}?quality=high&auth_key=expired"),
            "0123456789",
            1_700_000_000,
        )
        .unwrap();
        let url = reqwest::Url::parse(&signed).unwrap();
        let pairs: Vec<_> = url.query_pairs().collect();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], ("quality".into(), "high".into()));
        assert!(
            signed.ends_with("auth_key=1700000000-0123456789-0-9231480da32939f907d44671d01b262f")
        );
    }

    #[test]
    fn rejects_unexpected_origin_or_nonce() {
        for url in [
            "http://mc.kurogames.com/bg.mp4",
            "https://example.com/bg.mp4",
        ] {
            assert!(sign_wwmi_url(url, "0123456789", 1_700_000_000).is_err());
        }
        for nonce in ["1234", "012345678g"] {
            assert!(sign_wwmi_url(VIDEO_URL, nonce, 1_700_000_000).is_err());
        }
    }
}
